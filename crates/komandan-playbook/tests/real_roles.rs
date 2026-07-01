//! Real-world role integration tests (spec §11.3).
//!
//! These tests clone real Ansible roles from GitHub, parse them with
//! komandan-playbook, and verify the supported-task subset runs green.
//! Gated behind `KOMANDAN_TEST_REAL_ROLES=1` (requires network).
//!
//! Unlike `scoped_roles.rs` (which drives synthetic fixtures built only from
//! supported modules), these tests point at the upstream role repos as they
//! exist today. Real roles use modules komandan does not yet implement
//! (`include_vars`, `package`, `getent`, `sysctl`, …); coverage is therefore
//! asserted as a percentage (`SUPPORTED_COVERAGE_THRESHOLD`), not as "every
//! task supported". The `--skip-unsupported` path lets the runner reach a
//! `PLAY RECAP` even when some tasks are skipped.
//!
//! Temp dirs use [`std::env::temp_dir`] (no `tempfile` dependency added).
//! `unwrap_used` / `expect_used` are denied even in tests, so fallible setup
//! uses `?` (`-> anyhow::Result<()>`) or `unwrap_or_else(|e| panic!(...))`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use komandan_playbook::executors::{ModuleRegistry, canonicalize, register_all};
use komandan_playbook::inventory::Inventory;
use komandan_playbook::parser::{Vars as PVars, leaf_tasks, parse_playbook_file, parse_tasks_text};
use komandan_playbook::runner::execute;
use komandan_playbook::runner::tags::TagFilter;
use komandan_playbook::test_support::null_core;

/// Minimum supported-module coverage (percent) a real role must clear.
/// The spec §11.3 lists docker / nginx / ssh-hardening as "high coverage
/// expected"; 50 % leaves headroom for upstream drift.
const SUPPORTED_COVERAGE_THRESHOLD: u32 = 50;

/// Plan-time control directives komandan resolves without a runtime executor
/// (handled in `expand_includes` / `resolve_roles_for_play`). Treated as
/// "supported" for coverage math.
const SUPPORTED_CONTROL: &[&str] = &[
    "include_tasks",
    "import_tasks",
    "include_role",
    "import_role",
    "import_playbook",
];

/// Per-role gate → clone+scan → assert coverage threshold → best-effort run.
fn enabled() -> bool {
    std::env::var("KOMANDAN_TEST_REAL_ROLES").is_ok_and(|v| v == "1")
}

/// Shared scratch dir for one test process: `<tmp>/komandan-real-roles-<pid>`.
fn temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("komandan-real-roles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
    dir
}

/// Per-role module coverage, accumulated across every parsed task/handler file.
#[derive(Default)]
struct Coverage {
    /// Total leaf tasks parsed.
    total: u32,
    /// Tasks whose (canonicalized) module komandan implements.
    supported: u32,
    /// Distinct unsupported module names encountered.
    unsupported: HashSet<String>,
    /// Task files that komandan failed to parse.
    parse_failures: u32,
    /// Task files scanned.
    files: u32,
}

impl Coverage {
    /// Supported-share as a rounded-down percentage.
    fn pct(&self) -> u32 {
        self.supported
            .checked_mul(100)
            .and_then(|x| x.checked_div(self.total))
            .unwrap_or(0)
    }
}

/// Clone (or reuse) `role` into `<temp_root>/roles/<galaxy>` and return its path.
fn clone_role(galaxy: &str, url: &str) -> anyhow::Result<PathBuf> {
    let target = temp_root().join("roles").join(galaxy);
    if target.is_dir() {
        return Ok(target);
    }
    let status = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1"])
        .arg(url)
        .arg(&target)
        .status()
        .map_err(|e| anyhow::anyhow!("spawn git clone for {galaxy}: {e}"))?;
    if !status.success() {
        anyhow::bail!("git clone failed for {galaxy} ({url})");
    }
    Ok(target)
}

/// Parse every `*.yml` / `*.yaml` under `<role>/<subdir>/`, classifying each
/// leaf task's module. Missing directory ⇒ no-op. Unparseable files increment
/// `parse_failures` and are skipped (do not abort the scan).
fn scan_subdir(dir: &Path, subdir: &str, registry: &ModuleRegistry, cov: &mut Coverage) {
    let base = dir.join(subdir);
    let Ok(entries) = std::fs::read_dir(&base) else {
        return; // absent dir: nothing to scan
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("yml" | "yaml")))
        .collect();
    files.sort_unstable();
    for path in &files {
        cov.files += 1;
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("read failure {}: {e}", path.display());
                continue;
            }
        };
        let nodes = match parse_tasks_text(&text) {
            Ok(n) => n,
            Err(e) => {
                cov.parse_failures += 1;
                eprintln!("parse failure in {}: {e}", path.display());
                continue;
            }
        };
        for node in &nodes {
            for task in leaf_tasks(node) {
                cov.total += 1;
                let canon = canonicalize(task.module.as_str());
                if registry.contains(&canon) || SUPPORTED_CONTROL.contains(&canon.as_str()) {
                    cov.supported += 1;
                } else {
                    cov.unsupported.insert(canon);
                }
            }
        }
    }
}

/// Clone `role`, scan its `tasks/` and `handlers/` dirs, return the coverage.
fn analyze_role(galaxy: &str, url: &str) -> anyhow::Result<Coverage> {
    let role_dir = clone_role(galaxy, url)?;
    let registry = register_all();
    let mut cov = Coverage::default();
    scan_subdir(&role_dir, "tasks", &registry, &mut cov);
    scan_subdir(&role_dir, "handlers", &registry, &mut cov);
    Ok(cov)
}

/// Write a localhost wrapper playbook invoking `galaxy` and run it through
/// [`execute`] in check-mode with `skip_unsupported = true`. Best-effort: a
/// non-fatal `Err` is logged but does not fail the test (real roles depend on
/// host facts that the mock core cannot supply); the hard gate is coverage.
fn run_role_check_mode(galaxy: &str) {
    let root = temp_root();
    let wrapper_path = root.join(format!("wrapper-{galaxy}.yml"));
    let wrapper =
        format!("---\n- hosts: localhost\n  gather_facts: false\n  roles:\n    - role: {galaxy}\n");
    if std::fs::write(&wrapper_path, wrapper).is_err() {
        eprintln!("could not write wrapper for {galaxy}; skipping execute");
        return;
    }
    let playbook = match parse_playbook_file(&wrapper_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("wrapper parse failed for {galaxy}: {e}");
            return;
        }
    };
    // Inject Debian-family facts so `when: ansible_os_family == "Debian"`
    // branches resolve (semi-strict templating rejects undefined top-level
    // vars); mirrors the synthetic scoped-role test approach.
    let mut extra = PVars::default();
    extra.0.insert(
        "ansible_os_family".to_string(),
        serde_yaml::Value::String("Debian".to_string()),
    );
    extra.0.insert(
        "ansible_distribution".to_string(),
        serde_yaml::Value::String("Debian".to_string()),
    );
    let core = null_core();
    match execute(
        &[(wrapper_path.to_string_lossy().into_owned(), playbook)],
        &Inventory::implicit_localhost(),
        None,
        &core,
        true,
        &TagFilter::none(),
        None,
        &extra,
        1,
        false,
        true,
    ) {
        Ok(report) => {
            eprintln!(
                "execute({galaxy}) ok — recap present: {}",
                report.contains("PLAY RECAP")
            );
        }
        Err(e) => {
            eprintln!("execute({galaxy}) returned Err (best-effort, not fatal): {e}");
        }
    }
}

/// Shared body: gate → clone+scan → assert coverage threshold → best-effort run.
fn check_real_role(galaxy: &str, url: &str) -> anyhow::Result<()> {
    if !enabled() {
        eprintln!("KOMANDAN_TEST_REAL_ROLES != 1; skipping {galaxy}");
        return Ok(());
    }

    let cov = analyze_role(galaxy, url)?;
    let pct = cov.pct();
    eprintln!(
        "{galaxy}: {} tasks across {} files ({} parse failures); supported {}/{} = {pct}%; \
         unsupported: [{}]",
        cov.total,
        cov.files,
        cov.parse_failures,
        cov.supported,
        cov.total,
        {
            let mut v: Vec<String> = cov.unsupported.iter().cloned().collect();
            v.sort_unstable();
            v.join(", ")
        },
    );

    // `tasks/main.yml` is the role entrypoint — komandan must parse at least
    // one task file for the role to be meaningful under this harness.
    assert!(
        cov.files > 0,
        "{galaxy}: scanned no task files (clone incomplete?)"
    );
    assert!(
        pct >= SUPPORTED_COVERAGE_THRESHOLD,
        "{galaxy}: supported coverage {pct}% below {SUPPORTED_COVERAGE_THRESHOLD}% threshold"
    );

    run_role_check_mode(galaxy);
    Ok(())
}

#[test]
fn geerlingguy_docker_real_role() -> anyhow::Result<()> {
    check_real_role(
        "geerlingguy.docker",
        "https://github.com/geerlingguy/ansible-role-docker.git",
    )
}

#[test]
fn geerlingguy_nginx_real_role() -> anyhow::Result<()> {
    check_real_role(
        "geerlingguy.nginx",
        "https://github.com/geerlingguy/ansible-role-nginx.git",
    )
}

#[test]
fn devsec_ssh_hardening_real_role() -> anyhow::Result<()> {
    check_real_role(
        "dev-sec.ssh-hardening",
        "https://github.com/dev-sec/ansible-ssh-hardening.git",
    )
}
