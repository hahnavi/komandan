//! Phase 1 commands: `--syntax-check`, `--list-hosts`, `--list-tasks`.
//!
//! Each loads the inventory once, parses every positional playbook, and
//! renders a textual summary. Actual task execution lands in later phases; the
//! default (no listing flag) path returns a clear "not implemented" error so
//! `komandan playbook site.yml` is unambiguous about Phase 1 scope.

use std::fmt::Write;
use std::path::Path;

use komandan_plugin_abi::CoreApiRef;

use crate::cli::PlaybookArgs;
use crate::inventory::{self, Inventory};
use crate::parser::{self, HostMatcher, Play, Playbook, Task, leaf_tasks, play_leaf_tasks};
use crate::runner::tags::TagFilter;

/// Dispatch the command selected by the CLI flags.
///
/// Listing flags (`--syntax-check` / `--list-hosts` / `--list-tasks`) ignore
/// `core`; the default path runs the playbooks for real via [`crate::runner`].
///
/// # Errors
///
/// Propagates parse / inventory-load / execution failures.
pub fn run(opts: &PlaybookArgs, core: &CoreApiRef) -> anyhow::Result<String> {
    let inventory = inventory::load(opts.inventory.as_deref())?;

    let mut playbooks: Vec<(String, Playbook)> = Vec::with_capacity(opts.playbooks.len());
    for path in &opts.playbooks {
        let pb = parser::parse_playbook_file(Path::new(path))
            .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
        playbooks.push((path.clone(), pb));
    }

    if opts.syntax_check {
        return Ok(syntax_check_summary(&playbooks));
    }
    if opts.list_hosts {
        return Ok(render_list_hosts(
            &playbooks,
            &inventory,
            opts.limit.as_deref(),
        ));
    }
    if opts.list_tasks {
        return Ok(render_list_tasks(
            &playbooks,
            &inventory,
            opts.limit.as_deref(),
        ));
    }

    let tag_filter = TagFilter::from_cli(opts.tags.as_deref(), opts.skip_tags.as_deref());
    let extra_vars = parse_extra_vars(&opts.extra_vars)?;

    crate::runner::execute(
        &playbooks,
        &inventory,
        opts.limit.as_deref(),
        core,
        opts.check,
        &tag_filter,
        opts.start_at_task.as_deref(),
        &extra_vars,
        opts.forks,
        opts.diff,
        opts.skip_unsupported,
    )
}

/// Parse `-e`/`--extra-vars` CLI strings into a [`crate::parser::Vars`] map.
///
/// Each string may be:
/// - `key=value` (possibly multiple space-separated pairs)
/// - `@file.yml` (load YAML mapping from file)
/// - `{"key": "value"}` (inline JSON/YAML mapping)
///
/// # Errors
///
/// [`anyhow::Error`] on read or parse failure.
fn parse_extra_vars(raw: &[String]) -> anyhow::Result<crate::parser::Vars> {
    use crate::parser::Vars;
    let mut merged: indexmap::IndexMap<String, serde_yaml::Value> = indexmap::IndexMap::new();
    for item in raw {
        if let Some(path) = item.strip_prefix('@') {
            let text = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("failed to read extra-vars file {path}: {e}"))?;
            let vars = crate::parser::parse_vars_text(&text)
                .map_err(|e| anyhow::anyhow!("failed to parse extra-vars file {path}: {e}"))?;
            for (k, v) in vars.0 {
                merged.insert(k, v);
            }
        } else if item.starts_with('{') {
            let vars = crate::parser::parse_vars_text(item)
                .map_err(|e| anyhow::anyhow!("failed to parse extra-vars JSON: {e}"))?;
            for (k, v) in vars.0 {
                merged.insert(k, v);
            }
        } else {
            for token in item.split_whitespace() {
                if let Some((k, v)) = token.split_once('=') {
                    merged.insert(k.to_string(), serde_yaml::Value::String(v.to_string()));
                }
            }
        }
    }
    Ok(Vars(merged))
}

fn syntax_check_summary(playbooks: &[(String, Playbook)]) -> String {
    let total_plays: usize = playbooks.iter().map(|(_, pb)| pb.0.len()).sum();
    let mut total_tasks: usize = 0;
    for (path, pb) in playbooks {
        let base_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
        for play in &pb.0 {
            total_tasks += play_leaf_tasks(play).len();
            if !play.roles.is_empty()
                && let Ok(resolved) = crate::role::resolve_play_roles(base_dir, &play.roles)
            {
                for role in &resolved {
                    total_tasks += role
                        .tasks
                        .iter()
                        .map(|n| leaf_tasks(n).len())
                        .sum::<usize>();
                }
            }
        }
    }
    format!(
        "playbook syntax OK: {} file(s), {} play(s), {} task(s)",
        playbooks.len(),
        total_plays,
        total_tasks
    )
}

fn render_list_hosts(
    playbooks: &[(String, Playbook)],
    inv: &Inventory,
    limit: Option<&str>,
) -> String {
    let limit_set: Option<Vec<String>> = limit.map(|l| inv.resolve(l));
    let mut out = String::new();
    let mut play_no = 0;
    for (path, pb) in playbooks {
        let _ = writeln!(out, "playbook: {path}");
        for play in &pb.0 {
            play_no += 1;
            let mut hosts = inv.resolve(play_host_pattern(play));
            if let Some(lset) = &limit_set {
                hosts.retain(|h| lset.iter().any(|x| x == h));
            }
            let label = play_label(play);
            let _ = writeln!(out, "  play #{play_no} ({label}) ({} hosts)", hosts.len());
            for host in &hosts {
                let _ = writeln!(out, "    {host}");
            }
        }
    }
    out
}

fn render_list_tasks(
    playbooks: &[(String, Playbook)],
    inv: &Inventory,
    limit: Option<&str>,
) -> String {
    let limit_set: Option<Vec<String>> = limit.map(|l| inv.resolve(l));
    let mut out = String::new();
    let mut play_no = 0;
    for (path, pb) in playbooks {
        let _ = writeln!(out, "playbook: {path}");
        let base_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
        for play in &pb.0 {
            play_no += 1;
            let mut hosts = inv.resolve(play_host_pattern(play));
            if let Some(lset) = &limit_set {
                hosts.retain(|h| lset.iter().any(|x| x == h));
            }
            let label = play_label(play);
            let _ = writeln!(out, "  play #{play_no} ({label}): {} host(s)", hosts.len());

            let mut task_no = 0usize;
            // Execution order: pre_tasks → roles → tasks → post_tasks.
            for node in &play.pre_tasks {
                for task in leaf_tasks(node) {
                    task_no += 1;
                    write_task_entry(&mut out, task_no, task, None);
                }
            }
            // Role tasks (best-effort — skip silently if roles/ is absent).
            if !play.roles.is_empty()
                && let Ok(resolved) = crate::role::resolve_play_roles(base_dir, &play.roles)
            {
                for role in &resolved {
                    for node in &role.tasks {
                        for task in leaf_tasks(node) {
                            task_no += 1;
                            write_task_entry(&mut out, task_no, task, Some(&role.name));
                        }
                    }
                }
            }
            for node in &play.tasks {
                for task in leaf_tasks(node) {
                    task_no += 1;
                    write_task_entry(&mut out, task_no, task, None);
                }
            }
            for node in &play.post_tasks {
                for task in leaf_tasks(node) {
                    task_no += 1;
                    write_task_entry(&mut out, task_no, task, None);
                }
            }
        }
    }
    out
}

/// Write a single task entry for `--list-tasks`.
fn write_task_entry(out: &mut String, task_no: usize, task: &Task, role: Option<&str>) {
    let name = match (role, &task.name) {
        (Some(rn), Some(n)) => format!("{rn} : {n}"),
        (Some(rn), None) => format!("{rn} : (unnamed)"),
        (None, Some(n)) => n.clone(),
        (None, None) => "(unnamed)".to_string(),
    };
    let _ = writeln!(
        out,
        "    task #{task_no}  {name}  [{}]",
        task.module.as_str()
    );
    if !task.tags.is_empty() {
        let _ = writeln!(out, "    tags: [{}]", task.tags.join(", "));
    }
}

fn play_host_pattern(play: &Play) -> &str {
    play.hosts.as_ref().map_or("all", HostMatcher::as_str)
}

fn play_label(play: &Play) -> &str {
    play.name.as_deref().unwrap_or("(unnamed)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_playbook;

    fn sample_playbook() -> anyhow::Result<Playbook> {
        Ok(parse_playbook(
            r"
            - name: web setup
              hosts: webservers
              tasks:
                - name: install nginx
                  apt: { name: nginx, state: present }
                  tags: [pkg]
                - name: echo hi
                  command: echo hi
              post_tasks:
                - debug: msg=done
            ",
        )?)
    }

    #[test]
    fn syntax_check_counts_plays_and_tasks() -> anyhow::Result<()> {
        let pb = sample_playbook()?;
        let s = syntax_check_summary(&[("site.yml".to_string(), pb)]);
        assert!(s.contains("1 file(s)"), "{s}");
        assert!(s.contains("1 play(s)"), "{s}");
        // apt + command (tasks) + debug (post_tasks) = 3 leaf tasks
        assert!(s.contains("3 task(s)"), "{s}");
        Ok(())
    }

    #[test]
    fn list_tasks_renders_module_and_name() -> anyhow::Result<()> {
        let pb = sample_playbook()?;
        let mut inv = Inventory::default();
        inv.add_host_to_group("webservers", "w1");
        inv.add_host_to_group("webservers", "w2");
        let out = render_list_tasks(&[("site.yml".to_string(), pb)], &inv, None);
        assert!(out.contains("play #1"), "{out}");
        assert!(out.contains("install nginx"), "{out}");
        assert!(out.contains("[apt]"), "{out}");
        assert!(out.contains("[command]"), "{out}");
        assert!(out.contains("tags: [pkg]"), "{out}");
        Ok(())
    }

    #[test]
    fn list_hosts_resolves_group() -> anyhow::Result<()> {
        let pb = sample_playbook()?;
        let mut inv = Inventory::default();
        inv.add_host_to_group("webservers", "w1");
        inv.add_host_to_group("webservers", "w2");
        let out = render_list_hosts(&[("site.yml".to_string(), pb)], &inv, None);
        assert!(out.contains("(2 hosts)"), "{out}");
        assert!(out.contains("w1") && out.contains("w2"), "{out}");
        Ok(())
    }

    #[test]
    fn syntax_check_counts_role_tasks() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let base = dir.path();
        std::fs::create_dir_all(base.join("roles").join("myrole").join("tasks"))?;
        std::fs::write(
            base.join("roles")
                .join("myrole")
                .join("tasks")
                .join("main.yml"),
            "- name: role task 1\n  debug: msg=a\n- name: role task 2\n  debug: msg=b\n",
        )?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "---\n- hosts: localhost\n  roles:\n    - myrole\n",
        )?;
        let pb = parser::parse_playbook_file(&pb_path)?;
        let s = syntax_check_summary(&[(pb_path.to_string_lossy().into_owned(), pb)]);
        assert!(s.contains("2 task(s)"), "expected 2 role tasks: {s}");
        Ok(())
    }

    #[test]
    fn ansible_builtin_module_canonicalizes() -> anyhow::Result<()> {
        let pb = parse_playbook(
            r"
            - hosts: all
              tasks:
                - ansible.builtin.command: echo hi
            ",
        )?;
        let tasks = play_leaf_tasks(&pb.0[0]);
        assert_eq!(tasks[0].module.as_str(), "command");
        Ok(())
    }
}
