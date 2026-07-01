//! `block:`/`rescue:`/`always:` orchestration + handler flush (Phase 4).
//!
//! [`run_nodes`] walks a flat task list (the play's `pre_tasks`/`tasks`/...
//! or a handler list), descending into blocks recursively. A failure inside a
//! block's main tasks is caught by `rescue:` (Ansible semantics); `always:`
//! runs unconditionally except on an `end_play`. Notified handlers are collected
//! and flushed at `meta: flush_handlers` or at the play's end (see the runner).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use komandan_plugin_abi::prelude::*;

use crate::connection_pool::ConnectionPool;
use crate::executors::{FlowControl, ModuleRegistry, RuntimeInventory};
use crate::parser::{Block, TaskNode};
use crate::vars::{LayerKind, VarLayer, Vars};

use super::tags::TagFilter;
use super::task::{TaskStatus, eval_all_when, run_task};
use super::{Recap, tally, write_task_line};

/// Why the per-host task loop stopped (or kept going).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostStop {
    /// Keep processing this host.
    Continue,
    /// Stop this host (failure or `meta: end_host`).
    StopHost,
    /// Stop every host in the play (`meta: end_play`).
    StopPlay,
}

/// Per-host execution filter: tag selection (`--tags`/`--skip-tags`) +
/// `--start-at-task` state.
///
/// Threaded mutably through the recursive task walk so the `started` flag
/// updates as tasks are visited in order.
#[allow(clippy::struct_excessive_bools)] // mirrors run-time flags threaded through task walk
pub(super) struct RunFilter<'a> {
    /// Tag selection from `--tags`/`--skip-tags`.
    tags: &'a TagFilter,
    /// `--start-at-task` target name (if set).
    start_at: Option<&'a str>,
    /// Whether the start-at point has been reached (always `true` when
    /// `start_at` is `None`).
    started: bool,
    /// `any_errors_fatal:` — any task failure escalates to `StopPlay`.
    any_errors_fatal: bool,
    /// Shared set of `run_once:` task names already executed in this batch.
    /// When `None`, `run_once` tracking is disabled (e.g. handler flush).
    run_once_done: Option<&'a Arc<Mutex<HashSet<String>>>>,
    /// Directory to resolve runtime (templated) `include_tasks`/`import_tasks`
    /// against. `None` in handler-flush contexts (handlers don't include).
    base_dir: Option<&'a std::path::Path>,
    /// `--diff` mode: show file diffs.
    diff_mode: bool,
    /// Runtime inventory additions (`add_host` / `group_by`).
    runtime: &'a RuntimeInventory,
    /// Whether to skip unsupported modules with a warning (`--skip-unsupported`).
    skip_unsupported: bool,
}

impl<'a> RunFilter<'a> {
    /// Build a filter. When `start_at` is `None`, `started` begins `true`.
    /// `any_errors_fatal` escalates any task failure to `StopPlay`.
    /// `run_once_done` is the shared per-batch set of completed `run_once:`
    /// task names; pass `None` to disable `run_once` tracking.
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        tags: &'a TagFilter,
        start_at: Option<&'a str>,
        any_errors_fatal: bool,
        run_once_done: Option<&'a Arc<Mutex<HashSet<String>>>>,
        base_dir: Option<&'a std::path::Path>,
        diff_mode: bool,
        runtime: &'a RuntimeInventory,
        skip_unsupported: bool,
    ) -> Self {
        Self {
            tags,
            start_at,
            started: start_at.is_none(),
            any_errors_fatal,
            run_once_done,
            base_dir,
            diff_mode,
            runtime,
            skip_unsupported,
        }
    }

    /// A filter that admits everything by tag but skips the start-at check
    /// (for handler execution, which is not subject to `--start-at-task`).
    /// Handler flush is never subject to `any_errors_fatal` or `run_once`.
    const fn for_handlers(
        tags: &'a TagFilter,
        runtime: &'a RuntimeInventory,
        skip_unsupported: bool,
    ) -> Self {
        Self {
            tags,
            start_at: None,
            started: true,
            any_errors_fatal: false,
            run_once_done: None,
            base_dir: None,
            diff_mode: false,
            runtime,
            skip_unsupported,
        }
    }

    /// Atomically claim a `run_once` task for this host. Returns `true` if this
    /// caller is the first to claim it (and should run the task); `false` if
    /// another host in the batch already claimed it. Under parallel execution
    /// the claim is atomic (single `HashSet::insert` under a `Mutex` lock), so
    /// there is no TOCTOU race between checking and marking.
    fn claim_run_once(&self, task_name: &str) -> bool {
        self.run_once_done
            .is_none_or(|s| s.lock().is_ok_and(|mut g| g.insert(task_name.to_string())))
    }

    /// Whether a task named `name` with effective tags `tags` should run.
    /// Updates the `started` flag when `--start-at-task` is in use.
    fn admit(&mut self, name: Option<&str>, effective_tags: &[&str]) -> bool {
        if !self.started {
            if let Some(n) = name
                && let Some(target) = self.start_at
                && n == target
            {
                self.started = true;
            }
            if !self.started {
                return false;
            }
        }
        self.tags.should_run(effective_tags)
    }
}

/// Run a flat list of task nodes against one host, then honour an inline
/// `meta: flush_handlers`. Returns whether to keep going on this host.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_nodes(
    nodes: &[TaskNode],
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    notified: &mut Vec<String>,
    handlers: &[TaskNode],
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    filter: &mut RunFilter<'_>,
) -> HostStop {
    if let Some(stop) = run_each(
        nodes,
        host_label,
        host,
        registry,
        pool,
        vars,
        facts,
        flow,
        notified,
        out,
        recap,
        check_mode,
        filter,
        &[],
    ) {
        return stop;
    }
    // `meta: flush_handlers` raised mid-list: flush now, then continue.
    if take_flow(flow) == FlowControl::FlushHandlers {
        return flush_handlers(
            notified,
            handlers,
            host_label,
            host,
            registry,
            pool,
            vars,
            facts,
            flow,
            out,
            recap,
            check_mode,
            filter.tags,
            filter.runtime,
            filter.skip_unsupported,
        );
    }
    HostStop::Continue
}

/// Iterate `nodes`, returning the first [`HostStop`] that is not `Continue`.
#[allow(clippy::too_many_arguments)]
fn run_each(
    nodes: &[TaskNode],
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    notified: &mut Vec<String>,
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    filter: &mut RunFilter<'_>,
    parent_tags: &[String],
) -> Option<HostStop> {
    for node in nodes {
        let stop = match node {
            TaskNode::Task(t) => run_one_task(
                t.as_ref(),
                host_label,
                host,
                registry,
                pool,
                vars,
                facts,
                flow,
                notified,
                out,
                recap,
                check_mode,
                filter,
                parent_tags,
            ),
            TaskNode::Block(b) => run_block(
                b.as_ref(),
                host_label,
                host,
                registry,
                pool,
                vars,
                facts,
                flow,
                notified,
                out,
                recap,
                check_mode,
                filter,
                parent_tags,
            ),
        };
        if stop != HostStop::Continue {
            return Some(stop);
        }
    }
    None
}

/// Run one leaf task, report it, collect its `notify:` list.
#[allow(clippy::too_many_arguments)]
fn run_one_task(
    task: &crate::parser::Task,
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    notified: &mut Vec<String>,
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    filter: &mut RunFilter<'_>,
    parent_tags: &[String],
) -> HostStop {
    let task_name = task.name.as_deref().unwrap_or_else(|| task.module.as_str());

    // `run_once:` — atomically claim this task; if another host in the batch
    // already claimed it, skip. Claiming happens before execution so there is
    // no race under parallel `--forks` execution.
    if task.run_once == Some(true) && !filter.claim_run_once(task_name) {
        return HostStop::Continue;
    }

    // Templated `include_tasks`/`import_tasks` — paths containing `{{` could
    // not be resolved at plan time, so expand them now against per-host vars.
    let module_name = task.module.as_str();
    if (module_name == "include_tasks" || module_name == "import_tasks")
        && super::include_file_path(&task.args).contains("{{")
    {
        return expand_templated_include(
            task,
            module_name,
            host_label,
            host,
            registry,
            pool,
            vars,
            facts,
            flow,
            notified,
            out,
            recap,
            check_mode,
            filter,
        );
    }

    // Tag selection + `--start-at-task` (updates the started flag).
    let effective: Vec<&str> = task
        .tags
        .iter()
        .chain(parent_tags.iter())
        .map(String::as_str)
        .collect();
    if !filter.admit(task.name.as_deref(), &effective) {
        return HostStop::Continue;
    }

    let rec = match run_task(
        task,
        host_label,
        host,
        registry,
        pool,
        vars,
        facts,
        flow,
        check_mode,
        filter.diff_mode,
        filter.runtime,
        filter.skip_unsupported,
    ) {
        Ok(rec) => rec,
        Err(e) => {
            let _ = writeln!(out, "  [{host_label}] UNREACHABLE: {e}");
            recap.entry(host_label.to_string()).or_default().unreachable += 1;
            return HostStop::StopHost;
        }
    };
    write_task_line(out, &rec, host_label);
    tally(recap, host_label, rec.status);
    // Handlers are notified only on `changed` (Ansible semantics).
    if rec.status == TaskStatus::Changed {
        notified.extend(task.notify.iter().cloned());
    }
    // `any_errors_fatal:` — any failure escalates to a play-wide stop.
    let stop_play = rec.stop_play || (rec.status == TaskStatus::Failed && filter.any_errors_fatal);
    if stop_play {
        HostStop::StopPlay
    } else if rec.stop_host {
        HostStop::StopHost
    } else {
        HostStop::Continue
    }
}

/// Expand a templated `include_tasks`/`import_tasks` at runtime: render the
/// file path against per-host vars, load the file, parse it, and run the
/// sub-tasks inline via [`run_each`].
///
/// On file-load or parse errors the host is marked unreachable (matching the
/// existing pattern for `run_task` errors).
#[allow(clippy::too_many_arguments)]
fn expand_templated_include(
    task: &crate::parser::Task,
    module_name: &str,
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    notified: &mut Vec<String>,
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    filter: &mut RunFilter<'_>,
) -> HostStop {
    let raw_path = super::include_file_path(&task.args);

    // Render the path template against the current per-host vars.
    let rendered_path = match crate::templating::render_template(&raw_path, vars) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(e) => {
            let _ = writeln!(out, "  [{host_label}] ERROR rendering include path: {e}");
            recap.entry(host_label.to_string()).or_default().unreachable += 1;
            return HostStop::StopHost;
        }
    };

    let base = filter.base_dir.unwrap_or_else(|| std::path::Path::new("."));
    let path = base.join(&rendered_path);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(
                out,
                "  [{host_label}] ERROR: failed to {module_name} at {}: {e}",
                path.display()
            );
            recap.entry(host_label.to_string()).or_default().unreachable += 1;
            return HostStop::StopHost;
        }
    };
    let sub_nodes = match crate::parser::parse_tasks_text(&text) {
        Ok(nodes) => nodes,
        Err(e) => {
            let _ = writeln!(out, "  [{host_label}] ERROR parsing {module_name}: {e}");
            recap.entry(host_label.to_string()).or_default().unreachable += 1;
            return HostStop::StopHost;
        }
    };

    // Run the sub-tasks. `parent_tags` is empty: included tasks use their own
    // tags, not the includer's.
    run_each(
        &sub_nodes,
        host_label,
        host,
        registry,
        pool,
        vars,
        facts,
        flow,
        notified,
        out,
        recap,
        check_mode,
        filter,
        &[],
    )
    .unwrap_or(HostStop::Continue)
}

/// Run a `block:`/`rescue:`/`always:` aggregate.
#[allow(clippy::too_many_arguments)]
fn run_block(
    b: &Block,
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    notified: &mut Vec<String>,
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    filter: &mut RunFilter<'_>,
    parent_tags: &[String],
) -> HostStop {
    // Block-level `when:` — skip the whole block if false.
    if !b.when.is_empty() && !eval_all_when_ok(&b.when, vars) {
        return HostStop::Continue;
    }
    let pushed = push_block_vars(vars, b);

    // Effective tags for this block's children: parent's tags + block's tags.
    let block_tags: Vec<String> = parent_tags.iter().chain(b.tags.iter()).cloned().collect();

    let mut outcome = HostStop::Continue;
    let mut catchable_failure = false;

    // Main tasks: stop on first non-Continue; distinguish catchable failures.
    if let Some(stop) = run_each(
        b.tasks.as_slice(),
        host_label,
        host,
        registry,
        pool,
        vars,
        facts,
        flow,
        notified,
        out,
        recap,
        check_mode,
        filter,
        &block_tags,
    ) {
        if is_catchable_failure(stop, flow) {
            catchable_failure = true;
        } else {
            outcome = stop;
        }
    }

    // rescue absorbs a catchable failure.
    if catchable_failure && !b.rescue.is_empty() {
        // The failure is now handled: clear it so the block does not propagate
        // a host stop (Ansible: a rescued failure lets the play continue).
        catchable_failure = false;
        outcome = HostStop::Continue;
        if let Some(stop) = run_each(
            b.rescue.as_slice(),
            host_label,
            host,
            registry,
            pool,
            vars,
            facts,
            flow,
            notified,
            out,
            recap,
            check_mode,
            filter,
            &block_tags,
        ) {
            outcome = stop;
        }
    }

    // always runs unconditionally, except after an end_play.
    if outcome != HostStop::StopPlay
        && let Some(stop) = run_each(
            b.always.as_slice(),
            host_label,
            host,
            registry,
            pool,
            vars,
            facts,
            flow,
            notified,
            out,
            recap,
            check_mode,
            filter,
            &block_tags,
        )
    {
        outcome = stop;
    }

    if pushed {
        vars.pop();
    }
    // An unrescued failure propagates as a host stop.
    if catchable_failure && outcome == HostStop::Continue {
        HostStop::StopHost
    } else {
        outcome
    }
}

/// Flush notified handlers, in declaration order, deduped. Clears the notified
/// list.
#[allow(clippy::too_many_arguments)]
pub(super) fn flush_handlers(
    notified: &mut Vec<String>,
    handlers: &[TaskNode],
    host_label: &str,
    host: &HostInfo,
    registry: &ModuleRegistry,
    pool: &mut ConnectionPool<'_>,
    vars: &mut Vars,
    facts: &Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    flow: &Arc<Mutex<FlowControl>>,
    out: &mut String,
    recap: &mut IndexMap<String, Recap>,
    check_mode: bool,
    tag_filter: &TagFilter,
    runtime: &RuntimeInventory,
    skip_unsupported: bool,
) -> HostStop {
    let mut seen: HashSet<String> = HashSet::new();
    notified.retain(|n| seen.insert(n.clone()));
    if seen.is_empty() {
        return HostStop::Continue;
    }
    let _ = writeln!(out, "  RUNNING HANDLER");
    // Handlers are filtered by tags but NOT by `--start-at-task`.
    let mut filter = RunFilter::for_handlers(tag_filter, runtime, skip_unsupported);
    let mut overall = HostStop::Continue;
    for handler in handlers {
        if !handler_matches(handler, &seen) {
            continue;
        }
        let stop = match handler {
            TaskNode::Task(t) => run_one_task(
                t.as_ref(),
                host_label,
                host,
                registry,
                pool,
                vars,
                facts,
                flow,
                notified,
                out,
                recap,
                check_mode,
                &mut filter,
                &[],
            ),
            TaskNode::Block(b) => run_block(
                b.as_ref(),
                host_label,
                host,
                registry,
                pool,
                vars,
                facts,
                flow,
                notified,
                out,
                recap,
                check_mode,
                &mut filter,
                &[],
            ),
        };
        if stop != HostStop::Continue {
            overall = stop;
            break;
        }
    }
    notified.clear();
    overall
}

// ---- helpers ------------------------------------------------------------

/// Evaluate a block `when:` list, treating an evaluation error as false.
fn eval_all_when_ok(exprs: &[crate::parser::Expr], vars: &Vars) -> bool {
    match eval_all_when(exprs, vars) {
        Ok(v) => v,
        Err(e) => {
            // Surfaced elsewhere; a bad block-when just skips the block.
            let _ = e;
            false
        }
    }
}

/// Push the block's `vars:` onto the stack if non-empty; returns whether pushed.
fn push_block_vars(vars: &mut Vars, b: &Block) -> bool {
    if b.vars.0.is_empty() {
        return false;
    }
    vars.push(VarLayer::from_model_vars(LayerKind::BlockVars, &b.vars));
    true
}

/// Whether `stop` is a host failure catchable by `rescue:` (i.e. NOT a flow
/// directive like `end_host`/`end_play`).
fn is_catchable_failure(stop: HostStop, flow: &Arc<Mutex<FlowControl>>) -> bool {
    stop == HostStop::StopHost && !is_flow_stop(flow)
}

/// Whether the current flow directive is a hard stop (`end_host`/`end_play`).
fn is_flow_stop(flow: &Arc<Mutex<FlowControl>>) -> bool {
    matches!(
        current_flow(flow),
        FlowControl::EndHost | FlowControl::EndPlay
    )
}

/// Read the current flow signal.
fn current_flow(flow: &Arc<Mutex<FlowControl>>) -> FlowControl {
    flow.lock().map_or(FlowControl::Continue, |f| *f)
}

/// Read-and-reset: return the current flow, then set it to `Continue` if it was
/// `FlushHandlers` (the only transient signal).
fn take_flow(flow: &Arc<Mutex<FlowControl>>) -> FlowControl {
    let cur = current_flow(flow);
    if cur == FlowControl::FlushHandlers {
        reset_flow(flow, FlowControl::Continue);
    }
    cur
}

/// Overwrite the flow signal.
fn reset_flow(flow: &Arc<Mutex<FlowControl>>, val: FlowControl) {
    if let Ok(mut g) = flow.lock() {
        *g = val;
    }
}

/// Whether a handler node matches the notified set: by name, or (for tasks)
/// by `listen:` topic.
fn handler_matches(node: &TaskNode, notified: &HashSet<String>) -> bool {
    match node {
        TaskNode::Task(t) => {
            t.name.as_deref().is_some_and(|n| notified.contains(n))
                || t.listen.iter().any(|l| notified.contains(l))
        }
        TaskNode::Block(b) => b.name.as_deref().is_some_and(|n| notified.contains(n)),
    }
}
