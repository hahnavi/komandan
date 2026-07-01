//! Layered variable store for templating.
//!
//! Mirrors (a subset of) Ansible's variable precedence as an ordered stack of
//! immutable [`VarLayer`]s. Resolution walks the stack from the
//! highest-precedence layer down; [`Vars::flatten`] merges them low→high into a
//! single object handed to the templating engine as context.
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §7.1.
//!
//! Values are held as [`serde_json::Value`] because minijinja consumes serde
//! types natively; the parser's `serde_yaml::Value`s are converted at the layer
//! boundary via [`yaml_to_json`].

use indexmap::IndexMap;

use crate::parser::model;

/// One precedence layer in a [`Vars`] stack.
#[derive(Debug, Clone)]
pub struct VarLayer {
    /// Informational precedence label (the stack order is the real precedence).
    pub kind: LayerKind,
    map: IndexMap<String, serde_json::Value>,
}

/// Informational label for a variable layer.
///
/// The numeric ordering is **not** derived from this enum; the [`Vars`] stack
/// order is authoritative. This only exists for diagnostics and to drive
/// [`Vars::layer_from_kind`] lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub enum LayerKind {
    /// `roles/<r>/defaults/` — lowest real precedence.
    RoleDefaults,
    /// `register:` results from prior tasks.
    Registered,
    /// `set_fact:` (persistent across plays).
    SetFact,
    /// Magic vars (`inventory_hostname`, `ansible_host`, `play_hosts`, ...).
    Magic,
    /// Inventory-provided vars (group/host/inline).
    Inventory,
    /// `vars_files:`.
    PlayVarsFiles,
    /// `vars:` on a play.
    PlayVars,
    /// `roles/<r>/vars/`.
    RoleVars,
    /// `vars:` on a block.
    BlockVars,
    /// `vars:` on a task.
    TaskVars,
    /// `--extra-vars` / `-e` — highest precedence.
    ExtraVars,
}

impl VarLayer {
    /// Build an empty layer of the given kind.
    #[must_use]
    pub fn new(kind: LayerKind) -> Self {
        Self {
            kind,
            map: IndexMap::new(),
        }
    }

    /// Insert a value (overwrites a prior entry with the same key).
    pub fn insert(&mut self, key: impl Into<String>, value: serde_json::Value) -> &mut Self {
        self.map.insert(key.into(), value);
        self
    }

    /// Look up a key in this layer only.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.map.get(key)
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the layer holds no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Build a layer from a parser [`model::Vars`] block, converting every
    /// YAML value to JSON via [`yaml_to_json`].
    #[must_use]
    pub fn from_model_vars(kind: LayerKind, mv: &model::Vars) -> Self {
        let mut layer = Self::new(kind);
        for (k, v) in &mv.0 {
            layer.insert(k.clone(), yaml_to_json(v));
        }
        layer
    }
}

impl From<LayerKind> for VarLayer {
    fn from(kind: LayerKind) -> Self {
        Self::new(kind)
    }
}

/// The layered variable store.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    // Ordered lowest → highest precedence; later layers win on resolve/flatten.
    layers: Vec<VarLayer>,
}

impl Vars {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a layer on top (becomes the new highest precedence).
    pub fn push(&mut self, layer: VarLayer) -> &mut Self {
        self.layers.push(layer);
        self
    }

    /// Remove and return the top layer, if any.
    pub fn pop(&mut self) -> Option<VarLayer> {
        self.layers.pop()
    }

    /// Number of layers currently on the stack.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Resolve `key` against the stack, highest-precedence layer first.
    ///
    /// Returns the first defined value, or `None` if no layer defines it.
    #[must_use]
    pub fn resolve(&self, key: &str) -> Option<&serde_json::Value> {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| layer.map.get(key))
    }

    /// Merge every layer into a single object, low→high precedence (later keys
    /// overwrite earlier ones). This is the context shape handed to minijinja.
    #[must_use]
    pub fn flatten(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        for layer in &self.layers {
            for (k, v) in &layer.map {
                obj.insert(k.clone(), v.clone());
            }
        }
        serde_json::Value::Object(obj)
    }

    /// All keys defined across every layer (deduplicated, unordered).
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for layer in &self.layers {
            for k in layer.map.keys() {
                if seen.insert(k.as_str()) {
                    out.push(k.clone());
                }
            }
        }
        out
    }

    /// Record a `set_fact:` value (spec §7.1 level 12).
    ///
    /// Inserts into the topmost [`LayerKind::SetFact`] layer, lazily pushing one
    /// if none exists yet. These persist across plays within a run; execution
    /// (Phase 4) drives this — the plumbing lives here so the call site is a
    /// one-liner.
    pub fn set_fact(&mut self, key: impl Into<String>, value: serde_json::Value) {
        write_back(self, LayerKind::SetFact, key, value);
    }

    /// Record a `register:` result (spec §7.1 level 13).
    ///
    /// Like [`set_fact`](Self::set_fact) but for the registered-vars layer.
    pub fn register(&mut self, key: impl Into<String>, value: serde_json::Value) {
        write_back(self, LayerKind::Registered, key, value);
    }
}

/// Insert `value` under `key` into the topmost layer of `kind`, pushing a fresh
/// layer if none of that kind is on the stack yet.
fn write_back(vars: &mut Vars, kind: LayerKind, key: impl Into<String>, value: serde_json::Value) {
    let idx = if let Some(i) = vars.layers.iter().rposition(|l| l.kind == kind) {
        i
    } else {
        vars.layers.push(VarLayer::new(kind));
        vars.layers.len() - 1
    };
    vars.layers[idx].insert(key, value);
}

/// Convert a parser YAML value into a templating-context JSON value.
///
/// Handles every `serde_yaml::Value` variant; non-string mapping keys are
/// stringified via their `Display` form so the resulting JSON object is always
/// valid. Tagged values (`!!int`, ...) are unwrapped to their inner value.
#[must_use]
pub fn yaml_to_json(v: &serde_yaml::Value) -> serde_json::Value {
    match v {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_yaml::Value::Number(n) => number_to_json(n),
        serde_yaml::Value::String(s) => serde_json::Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => yaml_key_to_string(other),
                };
                obj.insert(key, yaml_to_json(val));
            }
            serde_json::Value::Object(obj)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(&t.value),
    }
}

/// Convert a templated JSON value back into a YAML value (for the post-render
/// task-args tree). The inverse of [`yaml_to_json`].
#[must_use]
pub fn json_to_yaml(v: &serde_json::Value) -> serde_yaml::Value {
    match v {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(*b),
        serde_json::Value::Number(n) => json_number_to_yaml(n),
        serde_json::Value::String(s) => serde_yaml::Value::String(s.clone()),
        serde_json::Value::Array(a) => {
            serde_yaml::Value::Sequence(a.iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(o) => {
            let mut map = serde_yaml::Mapping::new();
            for (k, val) in o {
                map.insert(serde_yaml::Value::String(k.clone()), json_to_yaml(val));
            }
            serde_yaml::Value::Mapping(map)
        }
    }
}

fn json_number_to_yaml(n: &serde_json::Number) -> serde_yaml::Value {
    if let Some(i) = n.as_i64() {
        return serde_yaml::Value::Number(i.into());
    }
    if let Some(u) = n.as_u64() {
        return serde_yaml::Value::Number(u.into());
    }
    if let Some(f) = n.as_f64() {
        return serde_yaml::Value::Number(serde_yaml::Number::from(f));
    }
    serde_yaml::Value::Null
}

fn number_to_json(n: &serde_yaml::Number) -> serde_json::Value {
    if let Some(i) = n.as_i64() {
        return serde_json::Value::Number(i.into());
    }
    if let Some(u) = n.as_u64() {
        return serde_json::Value::Number(u.into());
    }
    if let Some(f) = n.as_f64() {
        return serde_json::Number::from_f64(f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number);
    }
    serde_json::Value::Null
}

fn yaml_key_to_string(k: &serde_yaml::Value) -> String {
    match k {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => String::from("null"),
        // Complex keys (rare): render via YAML and trim the trailing newline.
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim_end()
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_walks_high_to_low() {
        let mut vars = Vars::new();
        vars.push(
            VarLayer::new(LayerKind::PlayVars)
                .insert("x", json!(1))
                .clone(),
        );
        vars.push(
            VarLayer::new(LayerKind::TaskVars)
                .insert("x", json!(2))
                .clone(),
        );
        assert_eq!(vars.resolve("x"), Some(&json!(2)));
    }

    #[test]
    fn flatten_low_to_high_merge() {
        let mut vars = Vars::new();
        vars.push(
            VarLayer::new(LayerKind::PlayVars)
                .insert("a", json!(1))
                .clone(),
        );
        vars.push(
            VarLayer::new(LayerKind::TaskVars)
                .insert("b", json!(2))
                .insert("a", json!(9))
                .clone(),
        );
        let flat = vars.flatten();
        assert_eq!(flat["a"], json!(9));
        assert_eq!(flat["b"], json!(2));
    }

    #[test]
    fn from_model_vars_converts_yaml() {
        let mut m = indexmap::IndexMap::new();
        m.insert("n".to_string(), serde_yaml::Value::Number(42.into()));
        m.insert(
            "s".to_string(),
            serde_yaml::Value::String(String::from("hi")),
        );
        let layer = VarLayer::from_model_vars(LayerKind::PlayVars, &model::Vars(m));
        assert_eq!(layer.get("n"), Some(&json!(42)));
        assert_eq!(layer.get("s"), Some(&json!("hi")));
    }

    #[test]
    fn yaml_to_json_roundtrips_nested() -> anyhow::Result<()> {
        let yaml: serde_yaml::Value = serde_yaml::from_str("a: 1\nb: [2, 3]\nc:\n  d: hi\n")?;
        let jv = yaml_to_json(&yaml);
        assert_eq!(jv["a"], json!(1));
        assert_eq!(jv["b"], json!([2, 3]));
        assert_eq!(jv["c"]["d"], json!("hi"));
        // Back to YAML.
        let back = json_to_yaml(&jv);
        assert_eq!(back.get("a"), Some(&serde_yaml::Value::Number(1.into())));
        Ok(())
    }

    #[test]
    fn missing_key_resolves_none() {
        let vars = Vars::new();
        assert_eq!(vars.resolve("nope"), None);
    }
}
