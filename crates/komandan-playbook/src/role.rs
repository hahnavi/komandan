//! Ansible role loader.
//!
//! Reads the standard `roles/<name>/` directory layout into a [`Role`] struct
//! and resolves transitive `meta/main.yml` `dependencies:` into run order via
//! DFS topological sort.
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §5 (roles). The role-ref parsing here mirrors
//! `parser::yaml::parse_roles` / `parse_one_role`, replicated inline because
//! those helpers are crate-private to `parser::yaml`.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_yaml::{Mapping, Value};

use crate::error::ParseError;
use crate::parser::{Expr, RoleRef, TaskNode, Vars, parse_tasks_text, parse_vars_text};

/// A loaded Ansible role: tasks, handlers, vars, defaults from `roles/<name>/`.
#[derive(Debug, Clone)]
pub struct Role {
    /// Role name (directory basename).
    pub name: String,
    /// Canonical path to the role directory.
    pub path: PathBuf,
    /// `tasks/main.yml` — the main task list. Empty if absent.
    pub tasks: Vec<TaskNode>,
    /// `handlers/main.yml` — handler tasks for notify. Empty if absent.
    pub handlers: Vec<TaskNode>,
    /// Merged `vars/*.yml` (`main.yml` first, then alphabetical others).
    pub vars: Vars,
    /// Merged `defaults/*.yml` (`main.yml` first, then alphabetical others).
    pub defaults: Vars,
    /// `meta/main.yml` `dependencies:` — other roles to run first.
    pub dependencies: Vec<RoleRef>,
}

impl Role {
    /// Load a role from `roles/<name>/` relative to `base_dir`.
    ///
    /// Reads the standard Ansible role layout: `tasks/main.yml`,
    /// `handlers/main.yml`, `vars/*.yml`, `defaults/*.yml`, and
    /// `meta/main.yml`. Optional files are tolerated — an absent `tasks/`
    /// yields an empty task list, absent `vars/`/`defaults/`/`meta/` yield
    /// empty vars/defaults/dependencies.
    ///
    /// # Errors
    ///
    /// [`ParseError::Load`] when the role directory is missing or a file
    /// cannot be read; [`ParseError::Yaml`] / [`ParseError::Task`] when a
    /// YAML document fails to parse.
    pub fn load(base_dir: &Path, name: &str) -> Result<Self, ParseError> {
        let path = base_dir.join("roles").join(name);
        if !path.is_dir() {
            return Err(ParseError::load(format!(
                "role not found: {name} at {}",
                path.display()
            )));
        }

        let tasks = load_task_file(&path.join("tasks").join("main.yml"))?;
        let handlers = load_task_file(&path.join("handlers").join("main.yml"))?;
        let vars = load_vars_dir(&path.join("vars"))?;
        let defaults = load_vars_dir(&path.join("defaults"))?;
        let dependencies = load_meta_deps(&path.join("meta").join("main.yml"))?;

        Ok(Self {
            name: name.to_string(),
            path,
            tasks,
            handlers,
            vars,
            defaults,
            dependencies,
        })
    }
}

/// Resolve a role and all its transitive dependencies into run order
/// (dependencies first, then the role itself). Uses DFS topological sort.
///
/// Deduplicates: a role named as a dependency of multiple roles is loaded
/// once and appears once, at its first post-order position. The top-level
/// `role_ref` is always last.
///
/// # Errors
///
/// [`ParseError::Load`] if any role in the chain cannot be loaded, or if a
/// circular dependency is detected.
pub fn resolve_role_chain(base_dir: &Path, role_ref: &RoleRef) -> Result<Vec<Role>, ParseError> {
    let mut order: Vec<Role> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    dfs(base_dir, role_ref, &mut order, &mut done, &mut stack)?;
    Ok(order)
}

/// Resolve all roles referenced in a play's `roles:` list into a flat,
/// ordered list (dependencies first, deduplicated across all chains).
///
/// Each top-level role ref is resolved independently via
/// [`resolve_role_chain`]; results are concatenated with cross-chain
/// deduplication (a role named as a dependency of multiple top-level roles
/// appears once, at its first post-order position).
///
/// # Errors
///
/// [`ParseError::Load`] if any role cannot be loaded or a cycle is detected.
pub fn resolve_play_roles(base_dir: &Path, roles: &[RoleRef]) -> Result<Vec<Role>, ParseError> {
    let mut all: Vec<Role> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for role_ref in roles {
        let chain = resolve_role_chain(base_dir, role_ref)?;
        for role in chain {
            if !seen.contains(&role.name) {
                seen.push(role.name.clone());
                all.push(role);
            }
        }
    }
    Ok(all)
}

/// DFS visitor for [`resolve_role_chain`].
///
/// `done` holds fully-resolved role names (skip on re-encounter → dedup);
/// `stack` holds the current DFS path (re-encounter ⇒ cycle).
fn dfs(
    base_dir: &Path,
    role_ref: &RoleRef,
    order: &mut Vec<Role>,
    done: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> Result<(), ParseError> {
    let name = &role_ref.role;
    if done.contains(name) {
        return Ok(());
    }
    if stack.contains(name) {
        return Err(ParseError::load(format!(
            "circular role dependency detected: {name}"
        )));
    }
    let role = Role::load(base_dir, name)?;
    stack.push(name.clone());
    for dep in &role.dependencies {
        dfs(base_dir, dep, order, done, stack)?;
    }
    stack.pop();
    order.push(role);
    done.push(name.clone());
    Ok(())
}

// ---- file loaders -------------------------------------------------------

/// Read + parse a task-list YAML file; absent file ⇒ empty vec.
///
/// # Errors
///
/// [`ParseError::Yaml`] / [`ParseError::Task`] on parse failure;
/// [`ParseError::Load`] if the file exists but cannot be read.
fn load_task_file(path: &Path) -> Result<Vec<TaskNode>, ParseError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_tasks_text(&text),
        Err(_) if !path.exists() => Ok(Vec::new()),
        Err(e) => Err(ParseError::load(format!(
            "failed to read {}: {e}",
            path.display()
        ))),
    }
}

/// Merge `vars/*.yml` (or `defaults/*.yml`) into one [`Vars`]: `main.yml`
/// first, then any other `*.yml` files sorted alphabetically by filename.
///
/// # Errors
///
/// [`ParseError::Load`] if the directory cannot be enumerated or a file
/// cannot be read; [`ParseError::Yaml`] / [`ParseError::Play`] on parse
/// failure. Absent directory ⇒ empty [`Vars`].
fn load_vars_dir(dir: &Path) -> Result<Vars, ParseError> {
    let mut merged = Vars::default();
    if !dir.is_dir() {
        return Ok(merged);
    }

    let main = dir.join("main.yml");
    if main.is_file() {
        merge_vars(&mut merged, &read_vars(&main)?);
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|e| ParseError::load(format!("cannot read {}: {e}", dir.display())))?;
    let mut others: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let p = entry
            .map_err(|e| ParseError::load(format!("read entry in {}: {e}", dir.display())))?
            .path();
        if p.extension().and_then(|x| x.to_str()) == Some("yml")
            && p.file_name() != Some(std::ffi::OsStr::new("main.yml"))
        {
            others.push(p);
        }
    }
    others.sort_unstable();
    for p in others {
        merge_vars(&mut merged, &read_vars(&p)?);
    }
    Ok(merged)
}

/// Read + parse a vars file into [`Vars`].
///
/// # Errors
///
/// [`ParseError::Load`] if unreadable; [`ParseError::Yaml`] / [`ParseError::Play`]
/// on parse failure.
fn read_vars(path: &Path) -> Result<Vars, ParseError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ParseError::load(format!("failed to read {}: {e}", path.display())))?;
    parse_vars_text(&text)
}

/// Merge `src` into `dst` (later keys overwrite earlier). Both are [`Vars`].
fn merge_vars(dst: &mut Vars, src: &Vars) {
    for (k, v) in &src.0 {
        dst.0.insert(k.clone(), v.clone());
    }
}

// ---- meta/main.yml dependency parsing -----------------------------------

/// Parse `meta/main.yml`'s `dependencies:` list into [`Vec<RoleRef>`].
/// Absent file / null doc / missing key ⇒ empty vec.
///
/// # Errors
///
/// [`ParseError::Load`] if the file exists but cannot be read;
/// [`ParseError::Yaml`] on malformed YAML.
fn load_meta_deps(path: &Path) -> Result<Vec<RoleRef>, ParseError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) if !path.exists() => return Ok(Vec::new()),
        Err(e) => {
            return Err(ParseError::load(format!(
                "failed to read {}: {e}",
                path.display()
            )));
        }
    };
    let doc: Value = serde_yaml::from_str(&text).map_err(ParseError::yaml)?;
    let Value::Mapping(mapping) = unwrap_tagged(&doc) else {
        return Ok(Vec::new());
    };
    Ok(map_get(mapping, "dependencies").map_or_else(Vec::new, parse_role_list))
}

/// Parse a `dependencies:` / `roles:` value into [`Vec<RoleRef>`].
fn parse_role_list(value: &Value) -> Vec<RoleRef> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items.iter().filter_map(parse_role_ref).collect(),
        Value::String(s) => vec![bare_role_ref(s.clone())],
        _ => Vec::new(),
    }
}

/// Parse a single role ref: a bare name or `{ role:, vars:, tags:, when: }`.
fn parse_role_ref(value: &Value) -> Option<RoleRef> {
    match unwrap_tagged(value) {
        Value::String(s) => Some(bare_role_ref(s.clone())),
        Value::Mapping(m) => {
            let role = map_get(m, "role").and_then(value_to_string)?;
            let vars = map_get(m, "vars")
                .map(parse_vars_mapping)
                .unwrap_or_default();
            let tags = map_get(m, "tags")
                .map(parse_string_list)
                .unwrap_or_default();
            let when = map_get(m, "when").map(parse_when_list).unwrap_or_default();
            Some(RoleRef {
                role,
                vars,
                tags,
                when,
            })
        }
        _ => None,
    }
}

/// Build a no-frills [`RoleRef`] from a bare role name.
fn bare_role_ref(name: String) -> RoleRef {
    RoleRef {
        role: name,
        vars: Vars::default(),
        tags: Vec::new(),
        when: Vec::new(),
    }
}

// ---- serde_yaml helpers (mirror parser::yaml internals) -----------------

/// Build [`Vars`] from a YAML mapping value.
fn parse_vars_mapping(value: &Value) -> Vars {
    let mut out = IndexMap::new();
    if let Value::Mapping(m) = unwrap_tagged(value) {
        for (k, v) in m {
            if let Value::String(key) = unwrap_tagged(k) {
                out.insert(key.clone(), v.clone());
            }
        }
    }
    Vars(out)
}

/// Parse a `tags:`-style value: a list, or a comma-separated string.
fn parse_string_list(value: &Value) -> Vec<String> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items.iter().filter_map(value_to_string).collect(),
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse a `when:` value: a single expression or a list of expressions.
fn parse_when_list(value: &Value) -> Vec<Expr> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items
            .iter()
            .filter_map(|v| value_to_string(v).map(Expr))
            .collect(),
        Value::Null => Vec::new(),
        other => value_to_string(other)
            .map(|s| vec![Expr(s)])
            .unwrap_or_default(),
    }
}

/// Coerce a scalar YAML value (string/number/bool) to a [`String`].
fn value_to_string(value: &Value) -> Option<String> {
    match unwrap_tagged(value) {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// String-keyed lookup on a YAML mapping (returns `None` for absent or null).
fn map_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    let v = mapping.get(Value::String(key.to_string()))?;
    if matches!(unwrap_tagged(v), Value::Null) {
        None
    } else {
        Some(v)
    }
}

/// Strip a `Value::Tagged` wrapper, if any (YAML tags like `!!str`).
fn unwrap_tagged(value: &Value) -> &Value {
    match value {
        Value::Tagged(inner) => unwrap_tagged(&inner.value),
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Write `contents` to `base/<rel>`, creating parent dirs as needed.
    fn write_file(base: &Path, rel: &str, contents: &str) -> std::io::Result<()> {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)
    }

    /// Build a bare [`RoleRef`] (no vars/tags/when) for a role name.
    fn role_ref(name: &str) -> RoleRef {
        bare_role_ref(name.to_string())
    }

    #[test]
    fn load_basic_role() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(
            base,
            "roles/foo/tasks/main.yml",
            "- command: echo one\n- command: echo two\n",
        )?;
        write_file(
            base,
            "roles/foo/handlers/main.yml",
            "- name: restart svc\n  command: systemctl restart svc\n",
        )?;
        let r = Role::load(base, "foo")?;
        assert_eq!(r.name, "foo");
        assert_eq!(r.tasks.len(), 2, "expected 2 tasks");
        assert_eq!(r.handlers.len(), 1, "expected 1 handler");
        assert!(r.dependencies.is_empty());
        Ok(())
    }

    #[test]
    fn load_role_with_vars_and_defaults() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/r/vars/main.yml", "a: 1\nb: 2\n")?;
        write_file(base, "roles/r/vars/extra.yml", "b: 20\nc: 30\n")?;
        write_file(base, "roles/r/defaults/main.yml", "x: 100\n")?;
        write_file(base, "roles/r/defaults/extra.yml", "y: 200\n")?;
        let r = Role::load(base, "r")?;

        // vars: a from main, b overwritten by extra (20), c from extra.
        assert_eq!(r.vars.0.len(), 3);
        assert_eq!(
            r.vars.0.get("a"),
            Some(&Value::Number(serde_yaml::Number::from(1_i64)))
        );
        assert_eq!(
            r.vars.0.get("b"),
            Some(&Value::Number(serde_yaml::Number::from(20_i64))),
            "extra.yml should overwrite main.yml's b"
        );
        assert!(r.vars.0.contains_key("c"));

        // defaults: x from main, y from extra.
        assert_eq!(r.defaults.0.len(), 2);
        assert!(r.defaults.0.contains_key("x"));
        assert!(r.defaults.0.contains_key("y"));
        Ok(())
    }

    #[test]
    fn load_role_missing_optional_files() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/only/tasks/main.yml", "- command: echo hi\n")?;
        let r = Role::load(base, "only")?;
        assert_eq!(r.tasks.len(), 1);
        assert!(r.handlers.is_empty());
        assert!(r.vars.0.is_empty());
        assert!(r.defaults.0.is_empty());
        assert!(r.dependencies.is_empty());
        Ok(())
    }

    #[test]
    fn load_nonexistent_role_errors() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let res = Role::load(tmp.path(), "nope");
        assert!(res.is_err(), "expected error for missing role, got {res:?}");
        let msg = match res {
            Err(ParseError::Load(m)) => m,
            other => panic!("expected ParseError::Load, got {other:?}"),
        };
        assert!(msg.contains("nope"), "error should name the role: {msg}");
        Ok(())
    }

    #[test]
    fn resolve_chain_with_dependencies() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/B/tasks/main.yml", "- command: echo b\n")?;
        write_file(base, "roles/A/tasks/main.yml", "- command: echo a\n")?;
        write_file(base, "roles/A/meta/main.yml", "dependencies:\n  - B\n")?;
        let chain = resolve_role_chain(base, &role_ref("A"))?;
        assert_eq!(
            chain.len(),
            2,
            "chain: {:?}",
            chain.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        assert_eq!(chain[0].name, "B", "dependency must come first");
        assert_eq!(chain[1].name, "A", "dependent role must come last");
        Ok(())
    }

    #[test]
    fn resolve_chain_detects_cycle() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/A/meta/main.yml", "dependencies: [B]\n")?;
        write_file(base, "roles/B/meta/main.yml", "dependencies: [A]\n")?;
        let res = resolve_role_chain(base, &role_ref("A"));
        assert!(
            matches!(res, Err(ParseError::Load(ref m)) if m.contains("circular")),
            "expected circular-dependency error, got {res:?}"
        );
        Ok(())
    }

    #[test]
    fn resolve_chain_deduplicates() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/A/meta/main.yml", "dependencies: [B, C]\n")?;
        write_file(base, "roles/B/meta/main.yml", "dependencies: [C]\n")?;
        write_file(base, "roles/C/tasks/main.yml", "- command: echo c\n")?;
        let chain = resolve_role_chain(base, &role_ref("A"))?;
        let c_count = chain.iter().filter(|r| r.name == "C").count();
        assert_eq!(
            c_count,
            1,
            "C should appear once: {:?}",
            chain.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        // A still last.
        assert_eq!(chain.last().map(|r| r.name.as_str()), Some("A"));
        Ok(())
    }

    #[test]
    fn resolve_chain_preserves_order() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(base, "roles/A/meta/main.yml", "dependencies: [B, C]\n")?;
        write_file(base, "roles/B/tasks/main.yml", "- command: echo b\n")?;
        write_file(base, "roles/C/tasks/main.yml", "- command: echo c\n")?;
        let chain = resolve_role_chain(base, &role_ref("A"))?;
        let names: Vec<&str> = chain.iter().map(|r| r.name.as_str()).collect();
        // DFS post-order: B first, then C, then A.
        assert_eq!(names, vec!["B", "C", "A"], "deps before dependent, A last");
        Ok(())
    }

    #[test]
    fn meta_deps_accept_mapping_form_with_vars_and_tags() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        write_file(
            base,
            "roles/A/meta/main.yml",
            "dependencies:\n  - role: B\n    vars:\n      k: v\n    tags: [t1]\n",
        )?;
        write_file(base, "roles/B/tasks/main.yml", "- command: echo b\n")?;
        let r = Role::load(base, "A")?;
        assert_eq!(r.dependencies.len(), 1);
        let dep = &r.dependencies[0];
        assert_eq!(dep.role, "B");
        assert_eq!(dep.tags, vec!["t1".to_string()]);
        assert!(dep.vars.0.contains_key("k"));
        Ok(())
    }
}
