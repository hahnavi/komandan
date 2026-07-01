//! Templating & variable resolution (spec §7).
//!
//! Two entry points:
//! - [`render_template`] — render one template string, returning a native JSON
//!   value (a sole `{{ expr }}` yields the expression's native type; anything
//!   else is string-interpolated).
//! - [`render_value`] — recursively render every templatable string in a YAML
//!   value tree (task args, `vars:`, ...), honouring the `omit` sentinel.

pub mod engine;
pub mod filters;
pub mod jtests;
pub mod lookups;
pub mod magic;

use minijinja::Environment;

use crate::vars::{Vars, json_to_yaml, yaml_to_json};
use magic::is_omit;

pub use magic::{MagicVars, is_omit as value_is_omit};

/// A templating failure.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// minijinja could not compile or render the expression/template.
    #[error("template render error: {0}")]
    Template(#[from] minijinja::Error),
    /// A rendered [`minijinja::Value`] could not be converted to JSON.
    #[error("value conversion error: {0}")]
    Convert(String),
}

/// Render a single template string against `vars`, returning the native value.
///
/// A string that is *exactly* one `{{ expr }}` (no surrounding text, no second
/// expression) yields the expression's native type — a dict/list/int/bool flows
/// through unstringified, matching Ansible's native-type templating. Everything
/// else is string-interpolated and returned as a JSON string.
///
/// # Errors
///
/// Fails on undefined variables (`SemiStrict`), syntax errors, or filter errors.
pub fn render_template(template: &str, vars: &Vars) -> Result<serde_json::Value, RenderError> {
    let env = engine::build_environment();
    let ctx = vars.flatten();
    render_str(template, &env, &ctx)
}

/// Recursively render every templatable string in a YAML value tree.
///
/// Walks mappings, sequences, and scalar strings; keys containing `{{` are also
/// rendered. A value that resolves to the `omit` sentinel is dropped from its
/// parent mapping (and filtered out of sequences).
///
/// # Errors
///
/// Propagates the first [`RenderError`] encountered.
pub fn render_value(
    value: &serde_yaml::Value,
    vars: &Vars,
) -> Result<serde_yaml::Value, RenderError> {
    let env = engine::build_environment();
    let ctx = vars.flatten();
    let jv = yaml_to_json(value);
    let rendered = render_json(&jv, &env, &ctx)?;
    Ok(json_to_yaml(&rendered))
}

fn render_str(
    s: &str,
    env: &Environment<'_>,
    ctx: &serde_json::Value,
) -> Result<serde_json::Value, RenderError> {
    if let Some(expr) = single_expression(s) {
        let val = env.compile_expression(expr)?.eval(ctx)?;
        serde_json::to_value(&val).map_err(|e| RenderError::Convert(e.to_string()))
    } else {
        let out = env.render_str(s, ctx)?;
        Ok(serde_json::Value::String(out))
    }
}

fn render_json(
    v: &serde_json::Value,
    env: &Environment<'_>,
    ctx: &serde_json::Value,
) -> Result<serde_json::Value, RenderError> {
    match v {
        serde_json::Value::String(s) => {
            if needs_template(s) {
                render_str(s, env, ctx)
            } else {
                Ok(v.clone())
            }
        }
        serde_json::Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                let r = render_json(x, env, ctx)?;
                if !is_omit(&r) {
                    out.push(r);
                }
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, val) in o {
                let rv = render_json(val, env, ctx)?;
                if is_omit(&rv) {
                    continue;
                }
                let rk = render_key(k, env, ctx)?;
                out.insert(rk, rv);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn render_key(
    k: &str,
    env: &Environment<'_>,
    ctx: &serde_json::Value,
) -> Result<String, RenderError> {
    if !needs_template(k) {
        return Ok(k.to_string());
    }
    match render_str(k, env, ctx)? {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

/// If `s` is exactly one `{{ expr }}` with no surrounding text and no nested
/// delimiters, return the inner expression; otherwise `None`.
fn single_expression(s: &str) -> Option<&str> {
    let t = s.trim();
    let inner = t.strip_prefix("{{")?.strip_suffix("}}")?;
    let inner = inner.trim();
    if inner.contains("{{") || inner.contains("}}") {
        return None;
    }
    Some(inner)
}

/// Cheap pre-check: does this string contain any template delimiter at all?
fn needs_template(s: &str) -> bool {
    s.contains("{{") || s.contains("{%")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::{LayerKind, VarLayer};

    fn vars_with(pairs: &[(&str, serde_json::Value)]) -> Vars {
        let mut layer = VarLayer::new(LayerKind::PlayVars);
        for (k, v) in pairs {
            layer.insert((*k).to_string(), v.clone());
        }
        let mut vars = Vars::new();
        vars.push(layer);
        vars
    }

    #[test]
    fn native_type_single_expression() -> anyhow::Result<()> {
        let vars = vars_with(&[("d", serde_json::json!({"a": 1}))]);
        let out = render_template("{{ d }}", &vars)?;
        assert_eq!(out, serde_json::json!({"a": 1}));
        Ok(())
    }

    #[test]
    fn string_interpolation_when_mixed() -> anyhow::Result<()> {
        let vars = vars_with(&[("n", serde_json::json!(5))]);
        let out = render_template("count={{ n }}", &vars)?;
        assert_eq!(out, serde_json::json!("count=5"));
        Ok(())
    }

    #[test]
    fn undefined_errors() {
        let vars = Vars::new();
        assert!(render_template("{{ missing | mandatory }}", &vars).is_err());
    }

    #[test]
    fn render_value_recurses() -> anyhow::Result<()> {
        let vars = vars_with(&[("h", serde_json::json!("web1"))]);
        let yaml: serde_yaml::Value = serde_yaml::from_str("name: install\ntarget: '{{ h }}'\n")?;
        let out = render_value(&yaml, &vars)?;
        assert_eq!(out["target"], serde_yaml::Value::String("web1".into()));
        Ok(())
    }

    #[test]
    fn render_value_native_list_arg() -> anyhow::Result<()> {
        let vars = vars_with(&[("items", serde_json::json!(["a", "b"]))]);
        let yaml: serde_yaml::Value = serde_yaml::from_str("loop: '{{ items }}'\n")?;
        let out = render_value(&yaml, &vars)?;
        assert!(matches!(out["loop"], serde_yaml::Value::Sequence(_)));
        Ok(())
    }

    #[test]
    fn omit_drops_key() -> anyhow::Result<()> {
        let mut vars = vars_with(&[("x", serde_json::json!(1))]);
        let mut magic = VarLayer::new(LayerKind::Magic);
        magic.insert("omit", magic::omit_value());
        vars.push(magic);
        let yaml: serde_yaml::Value = serde_yaml::from_str("a: 1\nb: '{{ omit }}'\n")?;
        let out = render_value(&yaml, &vars)?;
        assert!(out.get("a").is_some(), "a should remain");
        assert!(out.get("b").is_none(), "b should be omitted");
        Ok(())
    }

    #[test]
    fn non_templated_passes_through() -> anyhow::Result<()> {
        let vars = Vars::new();
        let yaml: serde_yaml::Value = serde_yaml::from_str("k: plain\nn: 42\n")?;
        let out = render_value(&yaml, &vars)?;
        assert_eq!(out["n"], serde_yaml::Value::Number(42.into()));
        Ok(())
    }
}
