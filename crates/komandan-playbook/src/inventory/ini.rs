//! Minimal INI inventory parser (`/etc/ansible/hosts` style).
//!
//! Supports: `[group]` host lines (with inline `key=value` vars),
//! `[group:vars]`, `[group:children]`, and bare host lines (assigned to
//! `all`). Comments (`#` / `;`) and blank lines are ignored. Every host is
//! also a member of `all` (Ansible parity). Value typing and YAML-inventory
//! are later phases.
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §5.

use serde_yaml::Value;

use crate::error::ParseError;
use crate::inventory::Inventory;

/// Which kind of entries a `[header]` section collects.
#[derive(Debug, Clone)]
enum Section {
    /// `[group]` — host entries (with optional inline vars).
    Hosts(String),
    /// `[group:vars]` — `key=value` group vars.
    Vars(String),
    /// `[group:children]` — member group names.
    Children(String),
}

/// Parse an INI inventory document.
///
/// # Errors
///
/// [`ParseError::Load`] for malformed lines (host entry without a name,
/// `[header]` that does not close, ...).
pub fn parse_ini(text: &str) -> Result<Inventory, ParseError> {
    let mut inv = Inventory::default();
    let mut current: Option<Section> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let lineno = idx + 1;

        if line.starts_with('[') {
            let header = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .map(str::trim)
                .ok_or_else(|| {
                    ParseError::load(format!("line {lineno}: unterminated `[section]` header"))
                })?;
            let section = parse_section_header(header, lineno)?;
            // Ensure the named group exists.
            match &section {
                Section::Hosts(g) | Section::Vars(g) | Section::Children(g) => {
                    inv.groups.entry(g.clone()).or_default();
                }
            }
            current = Some(section);
            continue;
        }

        let section = current
            .clone()
            .unwrap_or_else(|| Section::Hosts("all".to_string()));
        match section {
            Section::Hosts(group) => {
                let mut parts = line.split_whitespace();
                let host = parts.next().ok_or_else(|| {
                    ParseError::load(format!("line {lineno}: host entry is empty"))
                })?;
                inv.add_host_to_group(&group, host);
                // Every host is a member of `all`.
                if group != "all" {
                    inv.add_host_to_group("all", host);
                }
                // Inline `key=value` host vars.
                for kv in parts {
                    if let Some((k, v)) = kv.split_once('=')
                        && let Some(hvars) = inv.hosts.get_mut(host)
                    {
                        hvars
                            .0
                            .insert(k.trim().to_string(), Value::String(v.trim().to_string()));
                    }
                }
            }
            Section::Vars(group) => {
                if let Some((k, v)) = line.split_once('=') {
                    inv.groups
                        .entry(group)
                        .or_default()
                        .vars
                        .0
                        .insert(k.trim().to_string(), Value::String(v.trim().to_string()));
                } else {
                    return Err(ParseError::load(format!(
                        "line {lineno}: `[group:vars]` entry missing `=` (`{line}`)"
                    )));
                }
            }
            Section::Children(group) => {
                inv.add_child_group(&group, line);
            }
        }
    }

    Ok(inv)
}

fn parse_section_header(header: &str, lineno: usize) -> Result<Section, ParseError> {
    match header.split(':').collect::<Vec<_>>().as_slice() {
        [group] => Ok(Section::Hosts(group.to_string())),
        [group, kind] => match *kind {
            "vars" => Ok(Section::Vars(group.to_string())),
            "children" => Ok(Section::Children(group.to_string())),
            other => Err(ParseError::load(format!(
                "line {lineno}: unknown section kind `:{other}` (expected `:vars` or `:children`)"
            ))),
        },
        _ => Err(ParseError::load(format!(
            "line {lineno}: malformed section header `[{header}]`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_groups_hosts_and_inline_vars() -> anyhow::Result<()> {
        let text = "
        [web]
        web1 ansible_host=10.0.0.1
        web2

        [db]
        db1

        [all:vars]
        ansible_user=deploy
        ";
        let inv = parse_ini(text)?;
        assert_eq!(
            inv.resolve("web"),
            vec!["web1".to_string(), "web2".to_string()]
        );
        assert_eq!(inv.resolve("db"), vec!["db1".to_string()]);
        assert_eq!(inv.resolve("all").len(), 3, "all should contain every host");
        // inline host var
        let web1 = inv
            .hosts
            .get("web1")
            .ok_or_else(|| anyhow::anyhow!("web1 missing"))?;
        assert_eq!(
            web1.0.get("ansible_host").and_then(|v| v.as_str()),
            Some("10.0.0.1")
        );
        // group var on all
        let all_vars = &inv
            .groups
            .get("all")
            .ok_or_else(|| anyhow::anyhow!("all group"))?
            .vars;
        assert_eq!(
            all_vars.0.get("ansible_user").and_then(|v| v.as_str()),
            Some("deploy")
        );
        Ok(())
    }

    #[test]
    fn group_children_expand() -> anyhow::Result<()> {
        let text = "
        [web]
        w1
        [db]
        d1
        [cluster:children]
        web
        db
        ";
        let inv = parse_ini(text)?;
        let cluster = inv.resolve("cluster");
        assert!(cluster.contains(&"w1".to_string()));
        assert!(cluster.contains(&"d1".to_string()));
        assert_eq!(cluster.len(), 2);
        Ok(())
    }

    #[test]
    fn bare_hosts_land_in_all() -> anyhow::Result<()> {
        let inv = parse_ini("lonely1\nlonely2")?;
        let all = inv.resolve("all");
        assert!(all.contains(&"lonely1".to_string()));
        assert!(all.contains(&"lonely2".to_string()));
        Ok(())
    }
}
