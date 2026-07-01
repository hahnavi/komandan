//! Scoped-role integration tests (spec §11.3).
//!
//! Synthetic role fixtures reproducing the module combinations of
//! `geerlingguy.docker`, `geerlingguy.nginx`, and `dev-sec.ssh-hardening`.
//! Each role's per-task module set is validated against an allow-list
//! (`tests/fixtures/roles/<name>.toml`) and every allowed module is checked
//! against the live module registry; the role is then run end-to-end via
//! [`execute`] against a mock `CoreApi`.
//!
//! The v0.1 definition-of-done (spec §11.3): every scoped role's
//! supported-task subset runs green. The skip-with-warning path (for
//! unsupported modules in real-world roles) is deferred; these fixtures use
//! only supported modules, so no task is skipped.

use std::collections::HashSet;
use std::path::PathBuf;

use komandan_playbook::executors::{canonicalize, register_all};
use komandan_playbook::inventory::Inventory;
use komandan_playbook::parser::{Vars as PVars, leaf_tasks, parse_playbook_file, parse_tasks_text};
use komandan_playbook::runner::execute;
use komandan_playbook::runner::tags::TagFilter;
use komandan_playbook::test_support::null_core;

/// Root of the synthetic role/playbook fixtures (`tests/fixtures`).
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Per-role allow/skip config (spec §11.3: `tests/fixtures/roles/<name>.toml`).
/// `mimics` is documentation-only and ignored here.
#[derive(serde::Deserialize)]
struct RoleConfig {
    /// Modules the role's tasks/handlers use; each must be registered.
    allow: Vec<String>,
}

/// Read + parse a role's allow-list config.
///
/// # Panics
///
/// Panics if the file is missing or malformed (fixture-authoring error).
fn load_role_config(role: &str) -> RoleConfig {
    let path = fixtures_dir().join("roles").join(format!("{role}.toml"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Every (canonicalized) module name referenced by a role's `tasks/` and
/// `handlers/` `main.yml`.
fn role_modules(role: &str) -> HashSet<String> {
    let base = fixtures_dir().join("roles").join(role);
    let mut out = HashSet::new();
    for subdir in ["tasks", "handlers"] {
        let main = base.join(subdir).join("main.yml");
        if let Ok(text) = std::fs::read_to_string(&main)
            && let Ok(nodes) = parse_tasks_text(&text)
        {
            for node in &nodes {
                for task in leaf_tasks(node) {
                    out.insert(canonicalize(task.module.as_str()));
                }
            }
        }
    }
    out
}

/// Run `site-<role>.yml` via [`execute`] and return the textual report.
fn run_role(role: &str) -> anyhow::Result<String> {
    let pb_path = fixtures_dir().join(format!("site-{role}.yml"));
    let playbook = parse_playbook_file(&pb_path)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", pb_path.display()))?;
    // Mimic facts on a Debian host so the `when: ansible_os_family == "Debian"`
    // apt tasks execute rather than error on an undefined var (the templating
    // engine runs in UndefinedBehavior::SemiStrict, which rejects undefined
    // top-level vars).
    let mut extra = PVars::default();
    extra.0.insert(
        "ansible_os_family".to_string(),
        serde_yaml::Value::String("Debian".to_string()),
    );
    let core = null_core();
    execute(
        &[(pb_path.to_string_lossy().into_owned(), playbook)],
        &Inventory::implicit_localhost(),
        None,
        &core,
        false,
        &TagFilter::none(),
        None,
        &extra,
        1,
        false,
        false,
    )
}

/// Validate a role's allow-list vs. its actual module usage and the live
/// registry, then run the role and assert it finishes green (`failed=0`,
/// no `FAILED` task line).
fn assert_role_green(role: &str) -> anyhow::Result<()> {
    let cfg = load_role_config(role);
    let registry = register_all();

    // Every allowed module is supported (registered in this build).
    for m in &cfg.allow {
        assert!(
            registry.contains(m),
            "role {role}: allowed module {m:?} is not registered"
        );
    }
    // Every module the role actually uses is declared in its allow-list (no
    // surprise unsupported modules that would need skip-with-warning).
    for m in &role_modules(role) {
        assert!(
            cfg.allow.iter().any(|a| a == m),
            "role {role}: uses module {m:?} not declared in its allow-list"
        );
    }

    let report = run_role(role)?;
    assert!(
        !report.contains("FAILED"),
        "role {role} reported a failure:\n{report}"
    );
    assert!(
        report.contains("failed=0"),
        "role {role} did not finish with failed=0:\n{report}"
    );
    assert!(
        report.contains("PLAY RECAP"),
        "role {role}: missing PLAY RECAP:\n{report}"
    );
    Ok(())
}

#[test]
fn docker_role_runs_green() -> anyhow::Result<()> {
    assert_role_green("docker")
}

#[test]
fn nginx_role_runs_green() -> anyhow::Result<()> {
    assert_role_green("nginx")
}

#[test]
fn ssh_hardening_role_runs_green() -> anyhow::Result<()> {
    assert_role_green("ssh-hardening")
}
