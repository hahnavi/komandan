//! Inventory loading and host-pattern resolution.
//!
//! Phase 1 scope: inline `-i host1,host2,...` lists, implicit localhost (no
//! `-i`), and a basic INI inventory (`[group]`, `[group:vars]`,
//! `[group:children]`, inline `host var=val`). Group/host var *values* are
//! captured but not yet mapped onto komandan `Host` connection fields — that
//! is Phase 3. YAML inventory is Phase 5. `group_vars/`/`host_vars/`
//! directories adjacent to the inventory path are loaded in v0.1.
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §5.

pub mod ini;

use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::error::ParseError;
use crate::parser::{Vars, parse_vars_text};

/// A resolved inventory: ordered hosts + groups.
#[derive(Debug, Clone, Default)]
pub struct Inventory {
    /// Host name → host vars (inline + `host_vars`).
    pub hosts: IndexMap<String, Vars>,
    /// Group name → members (hosts + child groups) + group vars.
    pub groups: IndexMap<String, Group>,
}

/// A group's membership and vars.
#[derive(Debug, Clone, Default)]
pub struct Group {
    pub hosts: Vec<String>,
    pub children: Vec<String>,
    pub vars: Vars,
}

impl Inventory {
    /// Ansible's implicit localhost: a single host named `localhost` in the
    /// `all` group, used when no `-i` is given.
    #[must_use]
    pub fn implicit_localhost() -> Self {
        let mut inv = Self::default();
        inv.ensure_host("localhost");
        inv.add_host_to_group("all", "localhost");
        inv
    }

    /// Build an inventory from a comma-separated inline host list
    /// (`-i web1,web2,web3`).
    #[must_use]
    pub fn from_inline(spec: &str) -> Self {
        let mut inv = Self::default();
        for name in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            inv.ensure_host(name);
            inv.add_host_to_group("all", name);
        }
        inv
    }

    /// Insert a host if absent.
    pub fn ensure_host(&mut self, name: &str) {
        self.hosts.entry(name.to_string()).or_default();
    }

    /// Ensure a group exists in the inventory (created empty if absent).
    pub fn ensure_group(&mut self, name: &str) {
        self.groups.entry(name.to_string()).or_default();
    }

    /// Add a host to a group (creating both as needed). Dedupes.
    pub fn add_host_to_group(&mut self, group: &str, host: &str) {
        self.ensure_host(host);
        let g = self.groups.entry(group.to_string()).or_default();
        if !g.hosts.iter().any(|h| h == host) {
            g.hosts.push(host.to_string());
        }
    }

    /// Add a child group to a parent group.
    pub fn add_child_group(&mut self, parent: &str, child: &str) {
        let g = self.groups.entry(parent.to_string()).or_default();
        if !g.children.iter().any(|c| c == child) {
            g.children.push(child.to_string());
        }
    }

    /// Every host name in declaration order.
    #[must_use]
    pub fn all_host_names(&self) -> Vec<String> {
        self.hosts.keys().cloned().collect()
    }

    /// Resolve a host pattern to an ordered, de-duplicated host list.
    ///
    /// Supported (Phase 1): `all`, a group name (recursing through
    /// `children`), a bare host name, and comma-separated unions of the above.
    /// Negation (`!x`), regex (`~re`), and glob (`*pat`) are deferred.
    #[must_use]
    pub fn resolve(&self, pattern: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for token in pattern.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            for host in self.resolve_token(token) {
                if !out.iter().any(|h| h == &host) {
                    out.push(host);
                }
            }
        }
        out
    }

    fn resolve_token(&self, token: &str) -> Vec<String> {
        match token {
            "all" | "*" => self.all_host_names(),
            group if self.groups.contains_key(group) => self.group_hosts(group),
            host if self.hosts.contains_key(host) => vec![host.to_string()],
            // Unknown token: Ansible treats a bare unknown name as a host; we
            // surface nothing rather than inventing a phantom host.
            _ => Vec::new(),
        }
    }

    /// Hosts in a group, recursing through `children` (deduped).
    fn group_hosts(&self, group: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen_groups: Vec<String> = Vec::new();
        self.collect_group_hosts(group, &mut out, &mut seen_groups);
        out
    }

    fn collect_group_hosts(&self, group: &str, out: &mut Vec<String>, seen: &mut Vec<String>) {
        if seen.iter().any(|g| g == group) {
            return;
        }
        seen.push(group.to_string());
        if let Some(g) = self.groups.get(group) {
            for host in &g.hosts {
                if !out.iter().any(|h| h == host) {
                    out.push(host.clone());
                }
            }
            for child in &g.children {
                self.collect_group_hosts(child, out, seen);
            }
        }
    }
}

/// Load an inventory from a `-i` spec (or implicit localhost when `None`).
///
/// Detection: `None` ⇒ implicit localhost; a spec containing `,` ⇒ inline
/// host list; an existing file ⇒ INI parse; an existing directory ⇒ empty
/// inventory seeded by adjacent `group_vars/`/`host_vars/`; otherwise the spec
/// is treated as a single inline host.
///
/// When the inventory comes from a file or directory, adjacent
/// `group_vars/` and `host_vars/` directories are merged in (see
/// [`load_var_dirs`]).
///
/// # Errors
///
/// [`ParseError::Load`] if the file cannot be read or a vars file is
/// unreadable; [`ParseError`] from the INI parser or `parse_vars_text` on
/// malformed content.
pub fn load(spec: Option<&str>) -> Result<Inventory, ParseError> {
    let Some(spec) = spec else {
        return Ok(Inventory::implicit_localhost());
    };
    if spec.contains(',') {
        return Ok(Inventory::from_inline(spec));
    }
    let path = Path::new(spec);
    if path.exists() {
        let base_path = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or_else(|| Path::new("."))
        };
        let mut inv = if path.is_dir() {
            // Directory inventories: v0.1 only seeds from group_vars/host_vars.
            // Full multi-file inventory parsing is a later phase.
            Inventory::default()
        } else {
            let text = std::fs::read_to_string(path)
                .map_err(|e| ParseError::load(format!("failed to read inventory {spec}: {e}")))?;
            ini::parse_ini(&text)?
        };
        load_var_dirs(&mut inv, base_path)?;
        return Ok(inv);
    }
    Ok(Inventory::from_inline(spec))
}

/// Load `group_vars/` and `host_vars/` directories adjacent to `base_path`
/// and merge their vars into the inventory.
///
/// For each `group_vars/<name>.yml` (or `.yaml`), the vars are merged into
/// group `<name>`'s vars (creating the group if needed; `all` maps to the
/// `all` group). For each `host_vars/<name>.yml`, the vars are merged into
/// host `<name>`'s vars (creating the host if needed).
///
/// Files are sorted alphabetically; within a group/host, later files overwrite
/// earlier ones. Non-`.yml`/`.yaml` files are silently skipped. Missing
/// `group_vars/` or `host_vars/` directories are silently skipped (not an
/// error).
///
/// # Errors
///
/// [`ParseError::Load`] if a vars file exists but cannot be read;
/// [`ParseError`] from `parse_vars_text` on malformed YAML.
pub fn load_var_dirs(inv: &mut Inventory, base_path: &Path) -> Result<(), ParseError> {
    let group_dir = base_path.join("group_vars");
    if group_dir.is_dir() {
        for file in sorted_var_files(&group_dir)? {
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let text = read_vars_file(&file)?;
            let vars = parse_vars_text(&text)?;
            inv.ensure_group(&name);
            if let Some(g) = inv.groups.get_mut(&name) {
                g.vars.0.extend(vars.0);
            }
        }
    }

    let host_dir = base_path.join("host_vars");
    if host_dir.is_dir() {
        for file in sorted_var_files(&host_dir)? {
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let text = read_vars_file(&file)?;
            let vars = parse_vars_text(&text)?;
            inv.ensure_host(&name);
            if let Some(h) = inv.hosts.get_mut(&name) {
                h.0.extend(vars.0);
            }
        }
    }

    Ok(())
}

/// Collect `.yml`/`.yaml` entries in `dir`, sorted alphabetically by file name.
///
/// # Errors
///
/// [`ParseError::Load`] if the directory cannot be read.
fn sorted_var_files(dir: &Path) -> Result<Vec<PathBuf>, ParseError> {
    let mut files: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ParseError::load(format!("failed to read {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            ParseError::load(format!("failed to read entry in {}: {e}", dir.display()))
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_yaml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext == "yml" || ext == "yaml");
        if is_yaml {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Read a vars file to a string.
///
/// # Errors
///
/// [`ParseError::Load`] if the file cannot be read.
fn read_vars_file(path: &Path) -> Result<String, ParseError> {
    std::fs::read_to_string(path)
        .map_err(|e| ParseError::load(format!("failed to read {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_yaml::Value;
    use tempfile::TempDir;

    use super::*;

    /// Assert `vars[key]` equals the YAML string `exp`.
    fn assert_var_eq(vars: &Vars, key: &str, exp: &str) {
        match vars.0.get(key) {
            Some(Value::String(s)) => assert_eq!(s, exp, "var {key}"),
            other => panic!("var {key}: expected string {exp:?}, got {other:?}"),
        }
    }

    /// Write `contents` to `base/<rel>`, creating parent dirs as needed.
    fn write_file(base: &Path, rel: &str, contents: &str) -> std::io::Result<()> {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)
    }

    /// Load the inventory at `<base>/hosts`, surfacing non-UTF-8 paths as
    /// an error.
    fn load_hosts(base: &Path) -> anyhow::Result<Inventory> {
        let spec = base
            .join("hosts")
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 inventory path"))?
            .to_string();
        Ok(load(Some(&spec))?)
    }

    #[test]
    fn implicit_localhost_resolves_all_to_localhost() {
        let inv = Inventory::implicit_localhost();
        assert_eq!(inv.resolve("all"), vec!["localhost".to_string()]);
    }

    #[test]
    fn inline_list_populates_all_group() {
        let inv = Inventory::from_inline("web1, web2 ,web3");
        assert_eq!(inv.resolve("all").len(), 3);
        assert_eq!(inv.resolve("web2"), vec!["web2".to_string()]);
    }

    #[test]
    fn group_children_recurse() {
        let mut inv = Inventory::default();
        inv.add_host_to_group("web", "w1");
        inv.add_host_to_group("web", "w2");
        inv.add_host_to_group("db", "d1");
        inv.add_child_group("all", "web");
        inv.add_child_group("all", "db");
        let all = inv.resolve("all");
        assert_eq!(all.len(), 3, "all should recurse through children: {all:?}");
    }

    #[test]
    fn comma_union_dedupes() {
        let mut inv = Inventory::default();
        inv.add_host_to_group("web", "w1");
        inv.add_host_to_group("all", "w1");
        inv.add_host_to_group("all", "w2");
        let r = inv.resolve("w1,all");
        assert_eq!(r, vec!["w1".to_string(), "w2".to_string()]);
    }

    #[test]
    fn group_vars_all_loaded_into_all_group() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/all.yml", "foo: bar\n")?;
        write_file(tmp.path(), "hosts", "[webservers]\nweb1\nweb2\n")?;
        let inv = load_hosts(tmp.path())?;
        let g = inv
            .groups
            .get("all")
            .ok_or_else(|| anyhow::anyhow!("all group missing"))?;
        assert_var_eq(&g.vars, "foo", "bar");
        Ok(())
    }

    #[test]
    fn group_vars_named_group() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(
            tmp.path(),
            "group_vars/webservers.yml",
            "nginx_port: \"8080\"\n",
        )?;
        write_file(tmp.path(), "hosts", "[webservers]\nweb1\n")?;
        let inv = load_hosts(tmp.path())?;
        let g = inv
            .groups
            .get("webservers")
            .ok_or_else(|| anyhow::anyhow!("webservers group missing"))?;
        assert_var_eq(&g.vars, "nginx_port", "8080");
        Ok(())
    }

    #[test]
    fn host_vars_loaded() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "host_vars/web1.yml", "ansible_host: 10.0.0.1\n")?;
        write_file(tmp.path(), "hosts", "[webservers]\nweb1\n")?;
        let inv = load_hosts(tmp.path())?;
        let h = inv
            .hosts
            .get("web1")
            .ok_or_else(|| anyhow::anyhow!("web1 host missing"))?;
        assert_var_eq(h, "ansible_host", "10.0.0.1");
        Ok(())
    }

    #[test]
    fn yaml_extension_also_supported() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/all.yaml", "baz: qux\n")?;
        write_file(tmp.path(), "hosts", "[webservers]\nweb1\n")?;
        let inv = load_hosts(tmp.path())?;
        let g = inv
            .groups
            .get("all")
            .ok_or_else(|| anyhow::anyhow!("all group missing"))?;
        assert_var_eq(&g.vars, "baz", "qux");
        Ok(())
    }

    #[test]
    fn both_extensions_sorted() -> anyhow::Result<()> {
        // Lexical order: `all.yaml` < `all.yml` (`a` < `m`), so `.yml` is
        // processed last and overwrites `.yaml`.
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/all.yaml", "v: from_yaml\n")?;
        write_file(tmp.path(), "group_vars/all.yml", "v: from_yml\n")?;
        write_file(tmp.path(), "hosts", "[webservers]\nweb1\n")?;
        let inv = load_hosts(tmp.path())?;
        let g = inv
            .groups
            .get("all")
            .ok_or_else(|| anyhow::anyhow!("all group missing"))?;
        assert_var_eq(&g.vars, "v", "from_yml");
        Ok(())
    }

    #[test]
    fn missing_var_dirs_ok() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let mut inv = Inventory::default();
        inv.ensure_host("h1");
        // No group_vars/ or host_vars/ adjacent to base_path.
        load_var_dirs(&mut inv, tmp.path())?;
        assert_eq!(inv.hosts.len(), 1);
        assert!(inv.groups.is_empty());
        Ok(())
    }

    #[test]
    fn non_yaml_files_skipped() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/all.txt", "foo: bar\n")?;
        write_file(tmp.path(), "group_vars/all.md", "foo: bar\n")?;
        let mut inv = Inventory::default();
        load_var_dirs(&mut inv, tmp.path())?;
        // No .yml/.yaml → the `all` group is not created.
        assert!(inv.groups.get("all").is_none());
        Ok(())
    }

    #[test]
    fn load_with_directory_inventory() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/all.yml", "region: us-east\n")?;
        write_file(tmp.path(), "host_vars/web1.yml", "az: a\n")?;
        // Pass the directory itself as the `-i` spec.
        let spec = tmp
            .path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path"))?;
        let inv = load(Some(spec))?;
        let g = inv
            .groups
            .get("all")
            .ok_or_else(|| anyhow::anyhow!("all group missing"))?;
        assert_var_eq(&g.vars, "region", "us-east");
        let h = inv
            .hosts
            .get("web1")
            .ok_or_else(|| anyhow::anyhow!("web1 host missing"))?;
        assert_var_eq(h, "az", "a");
        Ok(())
    }

    #[test]
    fn load_var_dirs_empty_stem_skipped() -> anyhow::Result<()> {
        // A file whose stem is empty (e.g. `.yml`) must not create a "" group.
        let tmp = TempDir::new()?;
        write_file(tmp.path(), "group_vars/.yml", "foo: bar\n")?;
        let mut inv = Inventory::default();
        load_var_dirs(&mut inv, tmp.path())?;
        assert!(inv.groups.get("").is_none());
        Ok(())
    }

    #[test]
    fn ensure_group_creates_if_absent_and_idempotent() {
        let mut inv = Inventory::default();
        inv.ensure_group("g1");
        inv.ensure_group("g1");
        assert!(inv.groups.contains_key("g1"));
        assert_eq!(inv.groups.len(), 1);
    }
}
