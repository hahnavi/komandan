//! Ansible gap tests (spec §7.3) not already provided by minijinja.
//!
//! minijinja ships `defined`, `undefined`, `none`, `string`, `number`,
//! `mapping`, `sequence`, `iterable`, `boolean`, `odd`, `even`, `in`, etc. as
//! built-ins. This module registers the **remaining** tests: `dict` (a
//! `mapping` alias), the registered-result status tests (`success` /
//! `failure` / `changed` / `skipped`), the regex tests (`matches` / `search` /
//! `regex`), and `contains`.

use minijinja::{Environment, Value, value::ValueKind};

use super::filters::mj_to_json;

/// Register every gap test on `env`.
pub(super) fn register(env: &mut Environment<'_>) {
    env.add_test("dict", is_dict);
    env.add_test("success", is_success);
    env.add_test("failure", is_failure);
    env.add_test("changed", is_changed);
    env.add_test("skipped", is_skipped);
    env.add_test("matches", matches_start);
    env.add_test("search", search);
    env.add_test("regex", search);
    env.add_test("contains", contains);
}

fn is_dict(v: &Value) -> bool {
    v.kind() == ValueKind::Map
}

// --- registered-result status tests ---------------------------------------

/// `result is success` — true unless `failed` is explicitly `true`.
fn is_success(v: &Value) -> bool {
    !bool_field(v, "failed")
}

/// `result is failure` — true when `failed` is `true`.
fn is_failure(v: &Value) -> bool {
    bool_field(v, "failed")
}

/// `result is changed` — true when `changed` is `true`.
fn is_changed(v: &Value) -> bool {
    bool_field(v, "changed")
}

/// `result is skipped` — true when `skipped` is `true`.
fn is_skipped(v: &Value) -> bool {
    bool_field(v, "skipped")
}

fn bool_field(v: &Value, key: &str) -> bool {
    let Ok(serde_json::Value::Object(o)) = mj_to_json(v) else {
        return false;
    };
    o.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

// --- regex tests -----------------------------------------------------------

/// `value is matches(pattern)` — anchored-at-start match (Ansible `match`).
fn matches_start(v: &Value, pattern: &str) -> bool {
    let Some(s) = v.as_str() else { return false };
    regex::Regex::new(pattern).is_ok_and(|re| re.find(s).is_some_and(|m| m.start() == 0))
}

/// `value is search(pattern)` / `value is regex(pattern)` — partial match.
fn search(v: &Value, pattern: &str) -> bool {
    let Some(s) = v.as_str() else { return false };
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(s))
}

// --- contains --------------------------------------------------------------

/// `value is contains(item)` — substring (string) or membership (sequence).
fn contains(v: &Value, item: &Value) -> bool {
    if let Some(s) = v.as_str() {
        return item.as_str().is_some_and(|needle| s.contains(needle));
    }
    let Ok(serde_json::Value::Array(arr)) = mj_to_json(v) else {
        return false;
    };
    let Ok(target) = mj_to_json(item) else {
        return false;
    };
    arr.contains(&target)
}

#[cfg(test)]
mod tests {
    use crate::templating::engine::build_environment;

    #[test]
    fn dict_alias_works() {
        let env = build_environment();
        let out = env
            .render_str("{{ d is dict }}", serde_json::json!({"d": {"a": 1}}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, "true");
    }

    #[test]
    fn result_status_tests() {
        let env = build_environment();
        let ctx = serde_json::json!({"r": {"failed": false, "changed": true}});
        assert_eq!(
            env.render_str("{{ r is success }}", &ctx)
                .unwrap_or_default(),
            "true"
        );
        assert_eq!(
            env.render_str("{{ r is failure }}", &ctx)
                .unwrap_or_default(),
            "false"
        );
        assert_eq!(
            env.render_str("{{ r is changed }}", &ctx)
                .unwrap_or_default(),
            "true"
        );
    }

    #[test]
    fn matches_and_search() {
        let env = build_environment();
        let ctx = serde_json::json!({"s": "foobar"});
        assert_eq!(
            env.render_str("{{ s is matches('^foo') }}", &ctx)
                .unwrap_or_default(),
            "true"
        );
        assert_eq!(
            env.render_str("{{ s is matches('^bar') }}", &ctx)
                .unwrap_or_default(),
            "false"
        );
        assert_eq!(
            env.render_str("{{ s is search('oob') }}", &ctx)
                .unwrap_or_default(),
            "true"
        );
    }

    #[test]
    fn contains_string_and_seq() {
        let env = build_environment();
        assert_eq!(
            env.render_str(
                "{{ s is contains('ell') }}",
                serde_json::json!({"s": "hello"})
            )
            .unwrap_or_default(),
            "true"
        );
        assert_eq!(
            env.render_str(
                "{{ l is contains(2) }}",
                serde_json::json!({"l": [1, 2, 3]})
            )
            .unwrap_or_default(),
            "true"
        );
    }
}
