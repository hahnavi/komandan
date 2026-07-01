//! Per-task execution flow (spec §6.2): render args → look up executor →
//! honor `when` → run → `register` / `set_fact` write-back →
//! `changed_when` / `failed_when` → report.

use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use komandan_plugin_abi::prelude::*;
use minijinja::value::Value as MjValue;

use crate::connection_pool::ConnectionPool;
use crate::executors::{
    self, BecomeSettings, Connection, FlowControl, ModuleExecutor, ModuleRegistry,
    RuntimeInventory, TaskContext,
};
use crate::parser::{Expr, LoopSource, LoopSpec, Task};
use crate::templating::engine::build_environment;
use crate::vars::{LayerKind, VarLayer, Vars};

/// Per-task outcome status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Completed, no change.
    Ok,
    /// Completed and changed remote state.
    Changed,
    /// Failed.
    Failed,
    /// Skipped by `when`.
    Skipped,
}

/// A recorded task outcome.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    /// Resolved task name (or module name).
    pub task_name: String,
    /// Outcome status.
    pub status: TaskStatus,
    /// The module result.
    pub result: ModuleResult,
    /// Whether the host's play should stop after this task.
    pub stop_host: bool,
    /// Whether the whole play should stop after this task.
    pub stop_play: bool,
    /// Whether to suppress stdout/stderr in the output (Ansible `no_log: true`).
    pub no_log: bool,
    /// Per-item display info for looped tasks (`None` for non-looped tasks).
    pub loop_items: Option<Vec<LoopItemDisplay>>,
}

/// Per-item display info for looped tasks (used by `write_task_line`).
#[derive(Debug, Clone)]
pub struct LoopItemDisplay {
    /// Rendered label (or JSON of the item value when no label template).
    pub label: String,
    /// Whether this item's execution changed the target.
    pub changed: bool,
    /// Whether this item's execution failed.
    pub failed: bool,
}

/// The outcome of dispatching a task's body once or many times (for loops).
enum ExecOutcome {
    /// One execution (no loop).
    Single {
        result: ModuleResult,
        changed: bool,
        failed: bool,
    },
    /// One execution per loop iteration.
    Loop {
        /// Per-iteration results, in order.
        items: Vec<ModuleResult>,
        /// Per-iteration rendered labels (for display).
        labels: Vec<String>,
        changed: bool,
        failed: bool,
    },
}

impl ExecOutcome {
    /// A representative `ModuleResult` for display / `*_when` evaluation: the
    /// single result, or the last iteration's for a loop.
    fn representative(&self) -> (&ModuleResult, bool, bool) {
        match self {
            Self::Single {
                result,
                changed,
                failed,
            } => (result, *changed, *failed),
            Self::Loop {
                items,
                changed,
                failed,
                ..
            } => {
                let last = items.last().unwrap_or(&OK_RESULT);
                (last, *changed, *failed)
            }
        }
    }

    /// Per-iteration results if this was a loop.
    fn loop_items(&self) -> Option<&[ModuleResult]> {
        match self {
            Self::Loop { items, .. } => Some(items),
            Self::Single { .. } => None,
        }
    }
}

/// A reusable `ModuleResult::ok()` sentinel (loop representative when empty).
/// A `static` (not `const`) so `&OK_RESULT` is a true `'static` reference.
static OK_RESULT: ModuleResult = ModuleResult {
    changed: false,
    rc: 0,
    stdout: RString::new(),
    stderr: RString::new(),
    success: true,
    msg: ROption::RNone,
};

/// Execute one task against one host (the §6.2 flow).
///
/// Task-level failures (module error, unknown module, non-zero result) are
/// returned as [`TaskRecord`] with [`TaskStatus::Failed`] (respecting
/// `ignore_errors`); only catastrophic failures (pool/connection) propagate as
/// `Err`.
///
/// # Errors
///
/// `Err` on connection-pool failure or an internal error.
#[allow(clippy::too_many_arguments)]
pub fn run_task(
    task: &Task,
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    check_mode: bool,
    diff_mode: bool,
    runtime: &RuntimeInventory,
    skip_unsupported: bool,
) -> anyhow::Result<TaskRecord> {
    let task_name = task
        .name
        .clone()
        .unwrap_or_else(|| task.module.as_str().to_string());
    let core = pool.core();

    // 1. `when:` — skip if any condition is false (skips the whole loop too).
    if !task.when.is_empty() && !eval_all_when(&task.when, vars)? {
        core.report_record(
            leaked(task_name.as_str()),
            leaked(host_label),
            ReportStatus::Skipped,
        );
        return Ok(TaskRecord {
            task_name,
            status: TaskStatus::Skipped,
            result: ModuleResult::ok(),
            stop_host: false,
            stop_play: false,
            no_log: false,
            loop_items: None,
        });
    }

    // 2. Look up the executor once (fail fast on an unknown module).
    let Some(executor) = registry.lookup(task.module.as_str()) else {
        let canon = executors::canonicalize(task.module.as_str());
        if skip_unsupported {
            let warn_msg = format!(
                "task '{task_name}': module '{canon}' is not supported in this build — skipping"
            );
            core.log(LogLevel::Warn, leaked(&warn_msg));
            core.report_record(
                leaked(&task_name),
                leaked(host_label),
                ReportStatus::Skipped,
            );
            return Ok(TaskRecord {
                task_name,
                status: TaskStatus::Skipped,
                result: ModuleResult::ok(),
                stop_host: false,
                stop_play: false,
                no_log: task.no_log.unwrap_or(false),
                loop_items: None,
            });
        }
        let msg = format!(
            "module '{canon}' is not implemented in this build — see docs/ansible-compat.md"
        );
        let ignore = task.ignore_errors == Some(true);
        core.report_record(leaked(&task_name), leaked(host_label), ReportStatus::Failed);
        return Ok(TaskRecord {
            task_name,
            status: TaskStatus::Failed,
            result: ModuleResult::failure(1, msg),
            stop_host: !ignore,
            stop_play: false,
            no_log: task.no_log.unwrap_or(false),
            loop_items: None,
        });
    };
    let executor = executor.as_ref();

    // Check mode: skip modules that don't support it (mutating modules).
    // Control-flow modules (debug, ping, set_fact, assert, fail, meta, pause)
    // declare `supports_check_mode = true` and run normally.
    if check_mode && !executor.supports_check_mode() {
        core.report_record(
            leaked(task_name.as_str()),
            leaked(host_label),
            ReportStatus::Skipped,
        );
        return Ok(TaskRecord {
            task_name,
            status: TaskStatus::Skipped,
            result: ModuleResult::ok(),
            stop_host: false,
            stop_play: false,
            no_log: task.no_log.unwrap_or(false),
            loop_items: None,
        });
    }

    // 3. Dispatch single execution vs loop.
    let outcome = if let Some(spec) = &task.loop_ {
        run_loop(
            task, spec, executor, host_label, host, pool, vars, facts, flow, check_mode, diff_mode,
            runtime,
        )?
    } else {
        let (result, changed, failed) = exec_single(
            task, executor, host_label, host, pool, vars, facts, flow, check_mode, diff_mode,
            runtime,
        )?;
        ExecOutcome::Single {
            result,
            changed,
            failed,
        }
    };

    // 4. changed_when / failed_when (evaluated against snapshot + register var).
    let register = task.register.clone();
    let (rep_result, base_changed, base_failed) = outcome.representative();
    let changed = if task.changed_when.is_empty() {
        base_changed
    } else {
        eval_any(
            &task.changed_when,
            vars,
            register.as_deref().map(|n| (n, rep_result)),
        )?
    };
    let failed = if task.failed_when.is_empty() {
        base_failed
    } else {
        eval_any(
            &task.failed_when,
            vars,
            register.as_deref().map(|n| (n, rep_result)),
        )?
    };

    // 5. register: write a JSON view of the result (looped tasks get `results`).
    if let Some(name) = &register {
        vars.register(
            name,
            build_register_value(rep_result, changed, failed, outcome.loop_items()),
        );
    }

    // 6. Drain set_fact writes into the var store.
    drain_facts(facts, vars);

    // 7. Flow signal + failure stop semantics.
    let flow_val = flow.lock().map_or(FlowControl::Continue, |f| *f);
    let ignore = task.ignore_errors == Some(true);
    let stop_play = flow_val == FlowControl::EndPlay;
    let stop_host = flow_val == FlowControl::EndHost || (failed && !ignore);

    // 8. Report.
    let report_status = if failed {
        ReportStatus::Failed
    } else if changed {
        ReportStatus::Changed
    } else {
        ReportStatus::Ok
    };
    core.report_record(leaked(&task_name), leaked(host_label), report_status);

    let status = if failed {
        TaskStatus::Failed
    } else if changed {
        TaskStatus::Changed
    } else {
        TaskStatus::Ok
    };
    let (result, loop_items) = match outcome {
        ExecOutcome::Single { result, .. } => (result, None),
        ExecOutcome::Loop { items, labels, .. } => {
            let display: Vec<LoopItemDisplay> = labels
                .into_iter()
                .zip(items.iter())
                .map(|(label, item_result)| LoopItemDisplay {
                    label,
                    changed: item_result.changed,
                    failed: !item_result.success,
                })
                .collect();
            (combine_loop_results(&items, changed, failed), Some(display))
        }
    };
    Ok(TaskRecord {
        task_name,
        status,
        result,
        stop_host,
        stop_play,
        no_log: task.no_log.unwrap_or(false),
        loop_items,
    })
}

/// Combine per-iteration results into one reportable `ModuleResult`: stdout is
/// every iteration joined by newlines (so all loop output surfaces), `changed`
/// /`success` reflect the final evaluated flags.
fn combine_loop_results(items: &[ModuleResult], changed: bool, failed: bool) -> ModuleResult {
    let stdout = items
        .iter()
        .map(|r| r.stdout.as_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let stderr = items
        .iter()
        .map(|r| r.stderr.as_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let rc = items.iter().rev().find(|r| r.rc != 0).map_or(0, |r| r.rc);
    ModuleResult {
        changed,
        rc,
        stdout: RString::from(stdout),
        stderr: RString::from(stderr),
        success: !failed,
        msg: ROption::RNone,
    }
}

/// Render a task's args against `vars`, run the executor once, return the raw
/// result + changed/failed. Used for both the single path and each loop
/// iteration (the caller pushes an `item` layer for the latter).
#[allow(clippy::too_many_arguments)]
fn exec_single(
    task: &Task,
    executor: &dyn ModuleExecutor,
    host_label: &str,
    host: &HostInfo,
    pool: &mut ConnectionPool<'_>,
    vars: &Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    check_mode: bool,
    diff_mode: bool,
    runtime: &RuntimeInventory,
) -> anyhow::Result<(ModuleResult, bool, bool)> {
    let rendered_yaml = crate::templating::render_value(&task.args, vars)?;
    let args_json = expand_args(
        task.module.as_str(),
        &crate::vars::yaml_to_json(&rendered_yaml),
    );

    let snapshot = vars.flatten();
    let mut ctx = TaskContext::new(
        snapshot,
        Arc::clone(facts),
        Arc::clone(flow),
        std::sync::Arc::clone(runtime),
    );
    apply_task_settings(&mut ctx, task, check_mode);
    ctx.diff_mode = diff_mode;

    let conn: Connection<'_> = if let Some(delegate) = task.delegate_to.as_deref() {
        if delegate == "localhost" || delegate == "127.0.0.1" {
            let local_info = HostInfo {
                name: ROption::RSome(RStr::from("localhost")),
                address: RStr::from("127.0.0.1"),
                connection_type: RStr::from("local"),
                port: ROption::RNone,
                user: ROption::RNone,
                ssh_key_path: ROption::RNone,
                private_key_pass: ROption::RNone,
                password: ROption::RNone,
                become_method: ROption::RNone,
                become_user: ROption::RNone,
                elevate: ROption::RNone,
            };
            pool.acquire("localhost", local_info)?
        } else {
            // For non-localhost delegation, fall back to the original host
            // (full inventory-based delegation is a later enhancement).
            pool.acquire(host_label, host.clone())?
        }
    } else {
        pool.acquire(host_label, host.clone())?
    };
    let result = match executor.run(&conn, &args_json, &ctx) {
        Ok(r) => r,
        Err(e) => ModuleResult::failure(1, e.to_string()),
    };

    let changed = result.changed;
    let failed = !result.success;
    Ok((result, changed, failed))
}

/// Run a task once per loop item, pushing an `item` layer for each iteration.
#[allow(clippy::too_many_arguments)]
fn run_loop(
    task: &Task,
    spec: &LoopSpec,
    executor: &dyn ModuleExecutor,
    host_label: &str,
    host: &HostInfo,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    check_mode: bool,
    diff_mode: bool,
    runtime: &RuntimeInventory,
) -> anyhow::Result<ExecOutcome> {
    let source = render_loop_source(spec, vars)?;
    let iterations = expand_iterations(&source, spec);

    let mut items = Vec::with_capacity(iterations.len());
    let mut labels = Vec::with_capacity(iterations.len());
    let mut changed = false;
    let mut failed = false;
    for item_val in iterations {
        let mut layer = VarLayer::new(LayerKind::TaskVars);
        layer.insert("item", item_val.clone());
        vars.push(layer);

        // Render the `loop_control.label` template (or fall back to the item's
        // JSON representation) for per-iteration display.
        let label = task
            .loop_control
            .as_ref()
            .and_then(|lc| lc.label.as_deref())
            .map_or_else(
                || item_val.to_string(),
                |tmpl| {
                    crate::templating::render_template(tmpl, vars).map_or_else(
                        |_| item_val.to_string(),
                        |v| match v {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        },
                    )
                },
            );

        let (result, it_changed, it_failed) = exec_single(
            task, executor, host_label, host, pool, vars, facts, flow, check_mode, diff_mode,
            runtime,
        )?;

        vars.pop();

        changed |= it_changed;
        failed |= it_failed;
        items.push(result);
        labels.push(label);

        // A control-flow directive mid-loop stops the iteration.
        let flow_val = flow.lock().map_or(FlowControl::Continue, |f| *f);
        if matches!(flow_val, FlowControl::EndHost | FlowControl::EndPlay) {
            break;
        }
    }

    Ok(ExecOutcome::Loop {
        items,
        labels,
        changed,
        failed,
    })
}

/// Render the `loop:` / `with_*:` source to a JSON value (array or object).
/// A Jinja expression is evaluated against `vars`; an inline literal is
/// converted directly.
fn render_loop_source(spec: &LoopSpec, vars: &Vars) -> anyhow::Result<serde_json::Value> {
    let source = match spec {
        LoopSpec::Items(s) | LoopSpec::Dict(s) | LoopSpec::Indexed(s) => s,
    };
    match source {
        LoopSource::Expr(e) => {
            let bare = e
                .as_str()
                .trim()
                .strip_prefix("{{")
                .and_then(|s| s.strip_suffix("}}"))
                .map_or(e.as_str(), str::trim);
            let env = build_environment();
            let ctx_val = MjValue::from_serialize(vars.flatten());
            let val = env
                .compile_expression(bare)
                .map_err(|err| anyhow::anyhow!("invalid loop expression {bare:?}: {err}"))?
                .eval(&ctx_val)
                .map_err(|err| anyhow::anyhow!("loop expression evaluation failed: {err}"))?;
            Ok(serde_json::to_value(&val).unwrap_or(serde_json::Value::Null))
        }
        LoopSource::Literal(v) => Ok(crate::vars::yaml_to_json(v)),
    }
}

/// Turn a rendered loop `source` into the per-iteration `item` values.
fn expand_iterations(source: &serde_json::Value, spec: &LoopSpec) -> Vec<serde_json::Value> {
    match spec {
        LoopSpec::Items(_) => match source {
            serde_json::Value::Array(a) => a.clone(),
            other => vec![other.clone()],
        },
        LoopSpec::Indexed(_) => match source {
            // `item` = `[index, value]` (Ansible `with_indexed_items`).
            serde_json::Value::Array(a) => a
                .iter()
                .enumerate()
                .map(|(i, v)| serde_json::json!([i, v]))
                .collect(),
            other => vec![serde_json::json!([0, other])],
        },
        LoopSpec::Dict(_) => match source {
            // `item` = `{"key": k, "value": v}` (Ansible `with_dict`).
            serde_json::Value::Object(m) => m
                .iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect(),
            other => vec![other.clone()],
        },
    }
}

/// The JSON view of a result for `register:` (Ansible common fields). Looped
/// tasks additionally carry a per-iteration `results` array.
fn build_register_value(
    result: &ModuleResult,
    changed: bool,
    failed: bool,
    loop_items: Option<&[ModuleResult]>,
) -> serde_json::Value {
    let mut obj = result_json_map(result, changed, failed);
    if let Some(items) = loop_items {
        let results: Vec<serde_json::Value> = items
            .iter()
            .map(|r| result_json(r, r.changed, !r.success))
            .collect();
        obj.insert("results".to_string(), serde_json::Value::Array(results));
    }
    serde_json::Value::Object(obj)
}

/// Apply task-level settings onto the context.
fn apply_task_settings(ctx: &mut TaskContext, task: &Task, check_mode: bool) {
    ctx.check_mode = check_mode;
    ctx.no_log = task.no_log.unwrap_or(false);

    // Merge: task-level overrides play-level. Play-level become flows through
    // vars as `ansible_become` / `ansible_become_user` (injected by `run_host`).
    let play_become = ctx
        .vars
        .get("ansible_become")
        .and_then(|v| v.as_str())
        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
    let play_become_user = ctx
        .vars
        .get("ansible_become_user")
        .and_then(|v| v.as_str())
        .map(String::from);

    ctx.become_settings = BecomeSettings {
        enabled: task.r#become.unwrap_or(play_become),
        method: None,
        user: task.become_user.clone().or(play_become_user),
    };
    ctx.environment.clone_from(&task.environment);
}

/// Evaluate a list of Jinja `when`-style expressions; true iff ALL are truthy
/// (Ansible ANDs `when` entries).
pub(super) fn eval_all_when(exprs: &[Expr], vars: &Vars) -> anyhow::Result<bool> {
    if exprs.is_empty() {
        return Ok(true);
    }
    let env = build_environment();
    let ctx_val = MjValue::from_serialize(vars.flatten());
    for e in exprs {
        if !eval_expr(&env, e.as_str(), &ctx_val)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Evaluate `exprs` as a disjunction (true iff ANY is truthy). `extra`
/// injects a register var into the evaluation context.
fn eval_any(
    exprs: &[Expr],
    vars: &Vars,
    extra: Option<(&str, &ModuleResult)>,
) -> anyhow::Result<bool> {
    if exprs.is_empty() {
        return Ok(false);
    }
    let env = build_environment();
    let mut ctx_obj = match vars.flatten() {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if let Some((name, result)) = extra {
        ctx_obj.insert(
            name.to_string(),
            result_json(result, result.changed, !result.success),
        );
    }
    let ctx_val = MjValue::from_serialize(serde_json::Value::Object(ctx_obj));
    for e in exprs {
        if eval_expr(&env, e.as_str(), &ctx_val)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Evaluate one (possibly brace-wrapped) Jinja expression for truthiness.
fn eval_expr(env: &minijinja::Environment<'_>, expr: &str, ctx: &MjValue) -> anyhow::Result<bool> {
    let trimmed = expr.trim();
    let bare = trimmed
        .strip_prefix("{{")
        .and_then(|s| s.strip_suffix("}}"))
        .map_or(trimmed, str::trim);
    let val = env
        .compile_expression(bare)
        .map_err(|e| anyhow::anyhow!("invalid expression {trimmed:?}: {e}"))?
        .eval(ctx)
        .map_err(|e| anyhow::anyhow!("expression evaluation failed: {e}"))?;
    Ok(val.is_true())
}

/// Build the JSON view of a result for `register:` (Ansible common fields).
fn result_json(result: &ModuleResult, changed: bool, failed: bool) -> serde_json::Value {
    serde_json::Value::Object(result_json_map(result, changed, failed))
}

/// The map-building core of [`result_json`] (shared with loop aggregation).
fn result_json_map(
    result: &ModuleResult,
    changed: bool,
    failed: bool,
) -> serde_json::Map<String, serde_json::Value> {
    let mut obj = serde_json::Map::new();
    obj.insert("changed".to_string(), serde_json::Value::Bool(changed));
    obj.insert("failed".to_string(), serde_json::Value::Bool(failed));
    obj.insert(
        "success".to_string(),
        serde_json::Value::Bool(result.success),
    );
    obj.insert(
        "rc".to_string(),
        serde_json::Value::Number(result.rc.into()),
    );
    obj.insert(
        "stdout".to_string(),
        serde_json::Value::String(result.stdout.to_string()),
    );
    obj.insert(
        "stderr".to_string(),
        serde_json::Value::String(result.stderr.to_string()),
    );
    obj.insert(
        "stdout_lines".to_string(),
        serde_json::Value::Array(
            result
                .stdout
                .as_str()
                .lines()
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect(),
        ),
    );
    obj.insert(
        "stderr_lines".to_string(),
        serde_json::Value::Array(
            result
                .stderr
                .as_str()
                .lines()
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect(),
        ),
    );
    obj
}

/// Move the executor's `set_fact` writes into the per-host var store.
fn drain_facts(facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>, vars: &mut Vars) {
    let drained = facts
        .lock()
        .map(|mut f| f.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    for (k, v) in drained {
        vars.set_fact(k, v);
    }
}

/// Promote to `RStr<'static>` for report calls (process-bounded leak).
fn leaked(s: &str) -> RStr<'static> {
    crate::leak::rstr(s)
}

/// Free-form modules whose scalar args are a raw command (not `k=v` shorthand):
/// `command`, `shell`, `raw`, `script` (and their FQCN variants).
fn is_freeform(module: &str) -> bool {
    matches!(
        executors::canonicalize(module).as_str(),
        "command" | "shell" | "raw" | "script"
    )
}

/// Expand Ansible arg shorthand.
///
/// For non-freeform modules a scalar string is `key=value` pairs (with optional
/// quoting); for freeform modules a scalar (incl. bool/number) is the command
/// itself (`command: true` ⇒ `"true"`).
fn expand_args(module: &str, args: &serde_json::Value) -> serde_json::Value {
    match args {
        serde_json::Value::String(s) if !is_freeform(module) => parse_kv(s),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) if is_freeform(module) => {
            serde_json::Value::String(args.to_string())
        }
        _ => args.clone(),
    }
}

/// Parse a `key=value key=value` shorthand string into a JSON object. A bare
/// token (no `=`) becomes a boolean `true` flag (Ansible's convention). Values
/// may be single- or double-quoted.
fn parse_kv(s: &str) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for token in tokenize(s) {
        if let Some((k, v)) = token.split_once('=') {
            obj.insert(k.to_string(), serde_json::Value::String(unquote(v)));
        } else {
            obj.insert(token, serde_json::Value::Bool(true));
        }
    }
    serde_json::Value::Object(obj)
}

/// Split on whitespace, respecting single/double quotes (quotes are stripped
/// from the resulting tokens).
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Strip a single layer of matching surrounding quotes, if present.
fn unquote(s: &str) -> String {
    let t = s.trim();
    let bytes = t.as_bytes();
    if (bytes.first() == Some(&b'"') && bytes.last() == Some(&b'"'))
        || (bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\''))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn result_json_has_common_fields() {
        let r = ModuleResult {
            changed: true,
            rc: 0,
            stdout: RString::from("a\nb"),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        };
        let j = result_json(&r, true, false);
        assert_eq!(j["changed"], serde_json::Value::Bool(true));
        assert_eq!(j["failed"], serde_json::Value::Bool(false));
        assert_eq!(j["rc"], 0);
        assert_eq!(j["stdout"], "a\nb");
        assert_eq!(j["stdout_lines"].as_array().map_or(0, Vec::len), 2);
    }

    #[test]
    fn eval_expr_truthy_handles_braces() {
        let env = build_environment();
        let ctx = MjValue::from_serialize(serde_json::json!({}));
        assert!(eval_expr(&env, "1 == 1", &ctx).unwrap_or(false));
        assert!(eval_expr(&env, "{{ 1 == 1 }}", &ctx).unwrap_or(false));
        assert!(!eval_expr(&env, "false", &ctx).unwrap_or(true));
    }

    #[test]
    fn parse_kv_splits_key_value_pairs() {
        let v = parse_kv("name=nginx state=present");
        assert_eq!(v["name"], "nginx");
        assert_eq!(v["state"], "present");
    }

    #[test]
    fn parse_kv_handles_quoted_values() {
        let v = parse_kv("msg=\"hello world\"");
        assert_eq!(v["msg"], "hello world");
    }

    #[test]
    fn expand_args_freeform_keeps_scalar_command() {
        let v = expand_args("command", &serde_json::Value::Bool(true));
        assert_eq!(v, serde_json::Value::String("true".to_string()));
        let s = expand_args("shell", &serde_json::Value::String("echo hi".into()));
        assert_eq!(s, serde_json::Value::String("echo hi".to_string()));
    }

    #[test]
    fn expand_args_normal_module_splits_kv() {
        let v = expand_args("debug", &serde_json::Value::String("var=greeting".into()));
        assert_eq!(v["var"], "greeting");
    }

    #[test]
    fn expand_iterations_items_array() {
        let spec = LoopSpec::Items(LoopSource::Literal(serde_yaml::Value::Null));
        let items = expand_iterations(&serde_json::json!([1, 2, 3]), &spec);
        assert_eq!(items, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn expand_iterations_indexed_pairs() {
        let spec = LoopSpec::Indexed(LoopSource::Literal(serde_yaml::Value::Null));
        let items = expand_iterations(&serde_json::json!(["a", "b"]), &spec);
        assert_eq!(items, vec![json!([0, "a"]), json!([1, "b"])]);
    }

    #[test]
    fn expand_iterations_dict_pairs() {
        let spec = LoopSpec::Dict(LoopSource::Literal(serde_yaml::Value::Null));
        let items = expand_iterations(&serde_json::json!({"x": 1, "y": 2}), &spec);
        assert_eq!(items.len(), 2);
        assert!(items.contains(&json!({"key": "x", "value": 1})));
        assert!(items.contains(&json!({"key": "y", "value": 2})));
    }

    #[test]
    fn expand_iterations_scalar_wraps_single() {
        let spec = LoopSpec::Items(LoopSource::Literal(serde_yaml::Value::Null));
        let items = expand_iterations(&serde_json::json!("solo"), &spec);
        assert_eq!(items, vec![json!("solo")]);
    }

    #[test]
    fn combine_loop_results_joins_stdout() {
        let items = vec![
            ModuleResult {
                changed: true,
                rc: 0,
                stdout: RString::from("a"),
                stderr: RString::new(),
                success: true,
                msg: ROption::RNone,
            },
            ModuleResult {
                changed: true,
                rc: 0,
                stdout: RString::from("b"),
                stderr: RString::new(),
                success: true,
                msg: ROption::RNone,
            },
        ];
        let r = combine_loop_results(&items, true, false);
        assert_eq!(r.stdout.as_str(), "a\nb");
        assert!(r.changed);
        assert!(r.success);
    }

    #[test]
    fn build_register_value_loop_has_results() {
        let result = ModuleResult {
            changed: false,
            rc: 0,
            stdout: RString::new(),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        };
        let items = vec![result.clone(), result.clone()];
        let v = build_register_value(&result, true, false, Some(&items));
        assert_eq!(v["results"].as_array().map_or(0, Vec::len), 2);
        assert_eq!(v["changed"], serde_json::Value::Bool(true));
    }
}
