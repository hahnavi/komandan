//! Control-flow / controller-side executors that do no remote work (spec §6.3).
//!
//! Each is pure Rust: [`Debug`], [`Ping`], [`SetFact`], [`Assert`], [`Fail`],
//! [`Meta`], [`Pause`]. They ignore the pooled [`Connection`] (the runner still
//! acquires one — free for the implicit localhost target).

use komandan_plugin_abi::RString;
use komandan_plugin_abi::prelude::*;
use minijinja::value::Value as MjValue;

use super::{Connection, FlowControl, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register every control-flow executor on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(Debug);
    reg.register(Ping);
    reg.register(SetFact);
    reg.register(Assert);
    reg.register(Fail);
    reg.register(Meta);
    reg.register(Pause);
    reg.register(AddHost);
    reg.register(GroupBy);
}

/// Build a success result carrying `stdout`.
fn ok_with_stdout(stdout: &str) -> ModuleResult {
    ModuleResult {
        changed: false,
        rc: 0,
        stdout: RString::from(stdout),
        stderr: RString::new(),
        success: true,
        msg: ROption::RNone,
    }
}

/// `debug` — print a templated `msg:` or the value of a `var:`. Controller-side.
struct Debug;

impl ModuleExecutor for Debug {
    fn name(&self) -> &'static str {
        "debug"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let out = args.get("msg").map_or_else(
            || {
                args.get("var")
                    .and_then(serde_json::Value::as_str)
                    .map_or_else(String::new, |var| eval_var(var, &ctx.vars))
            },
            stringify_value,
        );
        Ok(ok_with_stdout(&out))
    }
}

/// `ping` — returns `pong`; connectivity was already proven by the pooled
/// connection's `create_connection`.
struct Ping;

impl ModuleExecutor for Ping {
    fn name(&self) -> &'static str {
        "ping"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        _args: &serde_json::Value,
        _ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let mut r = ok_with_stdout("pong");
        r.msg = ROption::RSome(RStr::from("pong"));
        Ok(r)
    }
}

/// `set_fact` — write each key into the per-host fact sink (drained by the
/// runner after the executor returns).
struct SetFact;

impl ModuleExecutor for SetFact {
    fn name(&self) -> &'static str {
        "set_fact"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let obj = args
            .as_object()
            .ok_or_else(|| ModuleError::args("set_fact expects a mapping of key: value"))?;
        let mut count = 0_usize;
        for (k, v) in obj {
            // `cacheable:` is a set_fact meta-key, not a fact.
            if k == "cacheable" {
                continue;
            }
            ctx.set_fact(k, v.clone());
            count += 1;
        }
        Ok(ok_with_stdout(&format!("set {count} fact(s)")))
    }
}

/// `assert` — evaluate a `that:` list of Jinja expressions; fail on the first
/// false one.
struct Assert;

impl ModuleExecutor for Assert {
    fn name(&self) -> &'static str {
        "assert"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let that = match args.get("that") {
            Some(v) => expression_list(v),
            None => return Err(ModuleError::args("assert requires a 'that:' list")),
        };
        let env = crate::templating::engine::build_environment();
        let ctx_val = MjValue::from_serialize(&ctx.vars);
        for expr in &that {
            if !eval_truthy(&env, expr, &ctx_val)? {
                let msg = args
                    .get("msg")
                    .and_then(serde_json::Value::as_str)
                    .map_or_else(|| format!("Assertion failed: {expr}"), str::to_string);
                return Ok(ModuleResult::failure(1, msg));
            }
        }
        Ok(ok_with_stdout(&format!(
            "asserted {} expression(s)",
            that.len()
        )))
    }
}

/// `fail` — always fail with an optional `msg:`.
struct Fail;

impl ModuleExecutor for Fail {
    fn name(&self) -> &'static str {
        "fail"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        _ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let msg = args
            .get("msg")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| "Failed as requested".to_string(), str::to_string);
        Ok(ModuleResult::failure(1, msg))
    }
}

/// `meta` — control-flow directives.
struct Meta;

impl ModuleExecutor for Meta {
    fn name(&self) -> &'static str {
        "meta"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let action = args
            .as_str()
            .or_else(|| args.get("action").and_then(serde_json::Value::as_str))
            .unwrap_or("noop");
        match action {
            "noop" => Ok(ok_with_stdout("noop")),
            "end_host" => {
                ctx.set_flow(FlowControl::EndHost);
                Ok(ok_with_stdout("end_host"))
            }
            "end_play" => {
                ctx.set_flow(FlowControl::EndPlay);
                Ok(ok_with_stdout("end_play"))
            }
            "flush_handlers" => {
                ctx.set_flow(FlowControl::FlushHandlers);
                Ok(ok_with_stdout("flush_handlers"))
            }
            // reset_connection / clear_facts / clear_host_errors are later work;
            // tolerated as no-ops here.
            other => Ok(ok_with_stdout(&format!(
                "meta:{other} (no-op in this build)"
            ))),
        }
    }
}

/// `pause` — non-interactive: sleep `seconds:` (default 0), echo a `prompt:`.
struct Pause;

impl ModuleExecutor for Pause {
    fn name(&self) -> &'static str {
        "pause"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        _ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| "Pausing".to_string(), str::to_string);
        if let Some(secs) = args.get("seconds").and_then(serde_json::Value::as_u64) {
            std::thread::sleep(std::time::Duration::from_secs(secs));
        }
        Ok(ok_with_stdout(&prompt))
    }
}

/// `add_host` — add a host to the in-memory inventory at runtime (visible to
/// subsequent plays via the shared [`RuntimeAdditions`] sink).
struct AddHost;

impl ModuleExecutor for AddHost {
    fn name(&self) -> &'static str {
        "add_host"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let name = args
            .get("name")
            .or_else(|| args.get("hostname"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ModuleError::args("add_host requires 'name'"))?;
        let groups: Vec<String> = match args.get("groups") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        };
        let mut host_vars = serde_json::Map::new();
        if let Some(obj) = args.as_object() {
            for (k, v) in obj {
                if k != "name" && k != "hostname" && k != "groups" {
                    host_vars.insert(k.clone(), v.clone());
                }
            }
        }
        let mut guard = ctx
            .runtime
            .lock()
            .map_err(|e| ModuleError::Other(format!("runtime lock: {e}")))?;
        guard.add_host(name, &groups, serde_json::Value::Object(host_vars));
        drop(guard);
        let mut r = ok_with_stdout(&format!("add_host: added '{name}'"));
        r.changed = true;
        Ok(r)
    }
}

/// `group_by` — create a dynamic group from a rendered key expression,
/// adding the current host to each comma-separated group name in `key:`.
struct GroupBy;

impl ModuleExecutor for GroupBy {
    fn name(&self) -> &'static str {
        "group_by"
    }

    fn supports_check_mode(&self) -> bool {
        true
    }

    fn run(
        &self,
        conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let key = args
            .get("key")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ModuleError::args("group_by requires 'key'"))?;
        let groups: Vec<&str> = key
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let host_name = conn
            .host()
            .name
            .as_ref()
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| "localhost".to_string());
        let mut guard = ctx
            .runtime
            .lock()
            .map_err(|e| ModuleError::Other(format!("runtime lock: {e}")))?;
        for g in &groups {
            guard.add_to_group(g, &host_name);
        }
        drop(guard);
        let mut r = ok_with_stdout(&format!("group_by: '{host_name}' -> {groups:?}"));
        r.changed = true;
        Ok(r)
    }
}

/// Render a JSON value for `debug` output: strings as-is, everything else as
/// compact JSON.
fn stringify_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Evaluate a `debug: var=X` expression against the variable snapshot.
///
/// Unlike a flat key lookup, this honours dotted access into registered
/// results (e.g. `var=out.stdout`). An undefined/invalid expression renders
/// Ansible's `VARIABLE IS NOT DEFINED!` message.
fn eval_var(var: &str, vars: &serde_json::Value) -> String {
    let env = crate::templating::engine::build_environment();
    let ctx_val = MjValue::from_serialize(vars);
    let bare = var
        .trim()
        .strip_prefix("{{")
        .and_then(|s| s.strip_suffix("}}"))
        .map_or_else(|| var.trim(), str::trim);
    env.compile_expression(bare)
        .and_then(|expr| expr.eval(&ctx_val))
        .map_or_else(
            |_| format!("VARIABLE IS NOT DEFINED!: {var}"),
            |v| {
                // SemiStrict mode evaluates a missing var to `Undefined` rather
                // than erroring; surface it the Ansible way.
                if v.is_undefined() {
                    format!("VARIABLE IS NOT DEFINED!: {var}")
                } else {
                    let json = serde_json::to_value(&v).unwrap_or(serde_json::Value::Null);
                    stringify_value(&json)
                }
            },
        )
}

/// Coerce a `that:` value into a list of expression strings.
fn expression_list(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Evaluate a (possibly brace-wrapped) Jinja expression for truthiness against
/// the task's variable snapshot.
fn eval_truthy(
    env: &minijinja::Environment<'_>,
    expr: &str,
    ctx: &MjValue,
) -> Result<bool, ModuleError> {
    let trimmed = expr.trim();
    let bare = trimmed
        .strip_prefix("{{")
        .and_then(|s| s.strip_suffix("}}"))
        .map_or(trimmed, str::trim);
    let val = env
        .compile_expression(bare)
        .map_err(|e| ModuleError::args(format!("invalid 'that' expression {trimmed:?}: {e}")))?
        .eval(ctx)
        .map_err(|e| ModuleError::Other(format!("'that' evaluation failed: {e}")))?;
    Ok(val.is_true())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{Connection, CoreApiRef};
    use crate::test_support::{localhost_host, null_core};
    use indexmap::IndexMap;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[allow(clippy::type_complexity)] // test-helper return bundle.
    fn ctx_with(
        vars: serde_json::Value,
    ) -> (
        TaskContext,
        Arc<Mutex<IndexMap<String, serde_json::Value>>>,
        Arc<Mutex<FlowControl>>,
    ) {
        let facts = TaskContext::empty_facts();
        let flow = TaskContext::default_flow();
        let ctx = TaskContext::new(
            vars,
            Arc::clone(&facts),
            Arc::clone(&flow),
            TaskContext::empty_runtime(),
        );
        (ctx, facts, flow)
    }

    fn conn(core: &CoreApiRef) -> Connection<'_> {
        Connection::new(core, ConnectionHandle::INVALID, localhost_host())
    }

    #[test]
    fn debug_msg_is_stringified() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Debug
            .run(&conn(&core), &json!({"msg": "hello"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "hello");
    }

    #[test]
    fn debug_var_looks_up_snapshot() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({"x": 42}));
        let r = Debug
            .run(&conn(&core), &json!({"var": "x"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "42");
    }

    #[test]
    fn debug_var_supports_dotted_access() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({"out": {"stdout": "hello"}}));
        let r = Debug
            .run(&conn(&core), &json!({"var": "out.stdout"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "hello");
    }

    #[test]
    fn debug_var_undefined_reports_clearly() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Debug
            .run(&conn(&core), &json!({"var": "missing"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            r.stdout.as_str().contains("VARIABLE IS NOT DEFINED"),
            "{}",
            r.stdout
        );
    }

    #[test]
    fn set_fact_writes_to_sink() {
        let core = null_core();
        let (ctx, facts, _) = ctx_with(json!({}));
        SetFact
            .run(
                &conn(&core),
                &json!({"foo": "bar", "n": 1, "cacheable": true}),
                &ctx,
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        let snapshot: IndexMap<String, serde_json::Value> =
            facts.lock().map(|g| g.clone()).unwrap_or_default();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot["foo"], json!("bar"));
    }

    #[test]
    fn assert_passes_on_truthy() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Assert
            .run(&conn(&core), &json!({"that": ["1 == 1", "true"]}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.success, "{}", r.stdout.as_str());
    }

    #[test]
    fn assert_fails_on_false() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Assert
            .run(&conn(&core), &json!({"that": "false"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.success);
    }

    #[test]
    fn fail_always_fails() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Fail
            .run(&conn(&core), &json!({"msg": "boom"}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.success);
        assert_eq!(r.stderr.as_str(), "boom");
    }

    #[test]
    fn ping_returns_pong() {
        let core = null_core();
        let (ctx, _, _) = ctx_with(json!({}));
        let r = Ping
            .run(&conn(&core), &json!({}), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "pong");
    }

    #[test]
    fn meta_end_host_raises_flow_signal() {
        let core = null_core();
        let (ctx, _, flow) = ctx_with(json!({}));
        Meta.run(&conn(&core), &json!("end_host"), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let signal = flow.lock().map_or(FlowControl::Continue, |f| *f);
        assert_eq!(signal, FlowControl::EndHost);
    }
}
