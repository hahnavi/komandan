//! Magic variables (spec §7.1 level 10) and the `omit` sentinel.
//!
//! Magic vars are synthesized by the runtime rather than read from inventory or
//! user input: `inventory_hostname`, `ansible_host`, `play_hosts`,
//! `playbook_dir`, `role_path`. They are injected as the lowest-priority real
//! layer (above facts/defaults) so user vars can still override them.
//!
//! `omit` is a special sentinel: when a templated value resolves to it, the
//! enclosing key is dropped from the rendered args tree (see
//! [`super::render_value`]).

use serde_json::{Value, json};

use crate::vars::{LayerKind, VarLayer};

/// Key used to mark the `omit` sentinel inside the templating context.
const OMIT_KEY: &str = "__komandan_omit__";

/// Inputs needed to build a magic-vars layer for a host within a play.
#[derive(Debug, Clone)]
pub struct MagicVars {
    /// The host's name in inventory.
    pub inventory_hostname: String,
    /// Connection address (defaults to `inventory_hostname`).
    pub ansible_host: Option<String>,
    /// Every host targeted by the current play.
    pub play_hosts: Vec<String>,
    /// Directory containing the playbook file.
    pub playbook_dir: Option<String>,
    /// Path to the current role, if any.
    pub role_path: Option<String>,
}

impl MagicVars {
    /// Build a magic-vars layer from the inputs.
    #[must_use]
    pub fn to_layer(&self) -> VarLayer {
        let mut layer = VarLayer::new(LayerKind::Magic);
        layer.insert("inventory_hostname", json!(self.inventory_hostname));
        layer.insert(
            "ansible_host",
            json!(
                self.ansible_host
                    .clone()
                    .unwrap_or_else(|| self.inventory_hostname.clone())
            ),
        );
        layer.insert("play_hosts", json!(self.play_hosts));
        if let Some(dir) = &self.playbook_dir {
            layer.insert("playbook_dir", json!(dir));
        }
        if let Some(rp) = &self.role_path {
            layer.insert("role_path", json!(rp));
        }
        layer.insert(OMIT_KEY, json!({ OMIT_KEY: true }));
        // `omit` is referenced by name; expose it as a JSON object marker the
        // render path recognizes (see [`omit_value`] / [`is_omit`]).
        layer.insert("omit", omit_value());
        layer
    }
}

/// The `omit` sentinel as a JSON marker.
#[must_use]
pub fn omit_value() -> Value {
    json!({ OMIT_KEY: true })
}

/// Whether a rendered JSON value is the `omit` sentinel.
#[must_use]
pub fn is_omit(v: &Value) -> bool {
    matches!(v, Value::Object(o) if o.len() == 1 && o.contains_key(OMIT_KEY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn magic_layer_keys() {
        let mv = MagicVars {
            inventory_hostname: "web1".into(),
            ansible_host: None,
            play_hosts: vec!["web1".into(), "web2".into()],
            playbook_dir: Some("/play".into()),
            role_path: None,
        };
        let layer = mv.to_layer();
        assert_eq!(layer.get("inventory_hostname"), Some(&json!("web1")));
        // ansible_host defaults to inventory_hostname.
        assert_eq!(layer.get("ansible_host"), Some(&json!("web1")));
        assert_eq!(layer.get("play_hosts"), Some(&json!(["web1", "web2"])));
        assert_eq!(layer.get("playbook_dir"), Some(&json!("/play")));
        assert!(layer.get("omit").is_some());
    }

    #[test]
    fn omit_round_trip() {
        let v = omit_value();
        assert!(is_omit(&v));
        assert!(!is_omit(&json!("not omit")));
    }
}
