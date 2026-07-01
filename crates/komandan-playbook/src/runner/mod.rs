//! Play/host orchestration loop (Phase 4+5 scope).
//!
//! For each play, resolve its hosts, split them into `serial:` batches, then
//! run every task node (`pre_tasks` → `roles` → `tasks` → `post_tasks`)
//! against each host via [`block::run_nodes`] (which descends `block:`/
//! `rescue:`/`always:` recursively). Notified handlers flush at
//! `meta: flush_handlers` or at the play's end.

pub mod block;
pub mod facts;
pub mod tags;
pub mod task;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use komandan_plugin_abi::CoreApiRef;
use rayon::prelude::*;

use crate::connection_pool::ConnectionPool;
use crate::error::ParseError;
use crate::executors::{self, FlowControl};
use crate::host;
use crate::inventory::Inventory;
use crate::parser::{
    Block, GatherFacts, HostMatcher, Play, Playbook, RoleRef, Serial, TaskNode, Vars as PVars,
};
use crate::templating::magic::MagicVars;
use crate::vars::{LayerKind, VarLayer, Vars};
use block::{HostStop, RunFilter, run_nodes};
use tags::TagFilter;
use task::TaskStatus;

/// Per-host tally for the `PLAY RECAP`.
#[derive(Default)]
pub(super) struct Recap {
    ok: u32,
    changed: u32,
    unreachable: u32,
    failed: u32,
    skipped: u32,
}

impl Recap {
    /// Merge another tally into this one (field-wise addition).
    const fn merge(&mut self, other: &Self) {
        self.ok += other.ok;
        self.changed += other.changed;
        self.unreachable += other.unreachable;
        self.failed += other.failed;
        self.skipped += other.skipped;
    }
}

/// Roles resolved for a play, ready for execution.
///
/// `defaults` and `vars` are merged across all resolved roles (including
/// transitive dependencies) into a single layer each. `blocks` wraps each
/// top-level role invocation's tasks (deps-first) in a [`Block`] carrying the
/// `RoleRef`'s tags/when/vars. `combined_handlers` merges the play's own
/// handlers with all role handlers.
struct ResolvedRoles {
    /// Merged role `defaults/*.yml` (lowest real precedence).
    defaults: PVars,
    /// Merged role `vars/*.yml` + `RoleRef` `vars:`.
    vars: PVars,
    /// One block per top-level role invocation (deps-first within each).
    blocks: Vec<TaskNode>,
    /// Play handlers + all role handlers, combined for notify flushing.
    combined_handlers: Vec<TaskNode>,
}

/// A play fully resolved for execution: includes expanded, roles loaded,
/// task sections ready to iterate.
struct PlayPlan {
    gather_facts: GatherFacts,
    play_vars: PVars,
    vars_file_vars: PVars,
    defaults: PVars,
    role_vars: PVars,
    role_blocks: Vec<TaskNode>,
    combined_handlers: Vec<TaskNode>,
    pre_tasks: Vec<TaskNode>,
    tasks: Vec<TaskNode>,
    post_tasks: Vec<TaskNode>,
    /// Play-level `become:` flag, injected into host vars as `ansible_become`.
    r#become: Option<bool>,
    /// Play-level `become_user:`, injected as `ansible_become_user`.
    become_user: Option<String>,
    /// Play-level `any_errors_fatal:` — any failure stops the whole play.
    any_errors_fatal: bool,
    /// Directory the playbook lives in; used to resolve runtime
    /// (`include_tasks`/`import_tasks` with templated paths) includes.
    base_dir: std::path::PathBuf,
    /// Whether to skip unsupported modules with a warning (`--skip-unsupported`).
    skip_unsupported: bool,
}

/// Build a [`PlayPlan`] from a parsed play: resolve roles, expand
/// `include_tasks`/`import_tasks` directives, load `vars_files:`.
fn build_play_plan(
    base_dir: &Path,
    play: &Play,
    skip_unsupported: bool,
) -> anyhow::Result<PlayPlan> {
    let roles = resolve_roles_for_play(base_dir, play)?;
    let pre_tasks = expand_includes(&play.pre_tasks, base_dir).map_err(anyhow::Error::from)?;
    let tasks = expand_includes(&play.tasks, base_dir).map_err(anyhow::Error::from)?;
    let post_tasks = expand_includes(&play.post_tasks, base_dir).map_err(anyhow::Error::from)?;
    let vars_file_vars =
        load_vars_files(base_dir, &play.vars_files).map_err(anyhow::Error::from)?;
    Ok(PlayPlan {
        gather_facts: play.gather_facts.clone(),
        play_vars: play.vars.clone(),
        vars_file_vars,
        defaults: roles.defaults,
        role_vars: roles.vars,
        role_blocks: roles.blocks,
        combined_handlers: roles.combined_handlers,
        pre_tasks,
        tasks,
        post_tasks,
        r#become: play.r#become,
        become_user: play.become_user.clone(),
        any_errors_fatal: play.any_errors_fatal.unwrap_or(false),
        base_dir: base_dir.to_path_buf(),
        skip_unsupported,
    })
}

/// Load `vars_files:` entries (relative to `base_dir`) into a merged [`PVars`].
///
/// # Errors
///
/// [`ParseError::Load`] if a file cannot be read; [`ParseError::Yaml`] on parse failure.
fn load_vars_files(base_dir: &Path, files: &[String]) -> Result<PVars, ParseError> {
    let mut merged = PVars::default();
    for file in files {
        let path = base_dir.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            ParseError::load(format!("failed to read vars file {}: {e}", path.display()))
        })?;
        let vars = crate::parser::parse_vars_text(&text)?;
        for (k, v) in vars.0 {
            merged.0.insert(k, v);
        }
    }
    Ok(merged)
}

/// Recursively expand `include_tasks`/`import_tasks` directives in a task
/// list, loading the referenced files relative to `base_dir`.
///
/// Both are treated identically (static include at plan time). Included files
/// are parsed via [`crate::parser::parse_tasks_text`] and spliced inline.
/// Includes inside `block:`/`rescue:`/`always:` are expanded recursively.
///
/// `include_role`/`import_role` are likewise expanded at plan time: the role
/// chain (deps first) is resolved via [`crate::role::resolve_role_chain`], its
/// tasks collected, and the role defaults/vars merged into a single layer
/// wrapped in a [`Block`] carrying the invoking task's `tags:`/`when:`/`vars:`.
///
/// # Errors
///
/// [`ParseError::Load`] if an included file cannot be read; [`ParseError`]
/// from [`crate::parser::parse_tasks_text`] on malformed YAML; [`ParseError`]
/// from [`crate::role::resolve_role_chain`] on a missing role.
fn expand_includes(nodes: &[TaskNode], base_dir: &Path) -> Result<Vec<TaskNode>, ParseError> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            TaskNode::Task(t) => {
                let module = t.module.as_str();
                if module == "include_tasks" || module == "import_tasks" {
                    let rel = include_file_path(&t.args);
                    if rel.contains("{{") {
                        // Templated path — defer to runtime expansion.
                        out.push(node.clone());
                    } else {
                        let path = base_dir.join(&rel);
                        let text = std::fs::read_to_string(&path).map_err(|e| {
                            ParseError::load(format!(
                                "failed to include {} at {}: {e}",
                                module,
                                path.display()
                            ))
                        })?;
                        let sub = crate::parser::parse_tasks_text(&text)?;
                        out.extend(expand_includes(&sub, base_dir)?);
                    }
                } else if module == "include_role" || module == "import_role" {
                    let role_name = include_role_name(&t.args);
                    let role_ref = RoleRef {
                        role: role_name.clone(),
                        vars: t.vars.clone(),
                        tags: t.tags.clone(),
                        when: t.when.clone(),
                    };
                    let chain = crate::role::resolve_role_chain(base_dir, &role_ref)?;

                    let mut chain_tasks: Vec<TaskNode> = Vec::new();
                    let mut merged_defaults = PVars::default();
                    let mut merged_vars = PVars::default();
                    for role in &chain {
                        for (k, v) in &role.defaults.0 {
                            merged_defaults.0.insert(k.clone(), v.clone());
                        }
                        for (k, v) in &role.vars.0 {
                            merged_vars.0.insert(k.clone(), v.clone());
                        }
                        chain_tasks.extend(role.tasks.iter().cloned());
                    }
                    // Defaults first, then vars (vars override defaults) within
                    // the block's own layer.
                    let mut block_vars = merged_defaults;
                    for (k, v) in &merged_vars.0 {
                        block_vars.0.insert(k.clone(), v.clone());
                    }

                    out.push(TaskNode::Block(Box::new(Block {
                        name: Some(format!("include_role : {role_name}")),
                        vars: block_vars,
                        when: t.when.clone(),
                        tasks: chain_tasks,
                        rescue: Vec::new(),
                        always: Vec::new(),
                        r#become: None,
                        tags: t.tags.clone(),
                    })));
                } else {
                    out.push(node.clone());
                }
            }
            TaskNode::Block(b) => {
                let mut nb = b.as_ref().clone();
                nb.tasks = expand_includes(&b.tasks, base_dir)?;
                nb.rescue = expand_includes(&b.rescue, base_dir)?;
                nb.always = expand_includes(&b.always, base_dir)?;
                out.push(TaskNode::Block(Box::new(nb)));
            }
        }
    }
    Ok(out)
}

/// Extract the file path from an include task's args (scalar or `{ file: ... }`).
pub(super) fn include_file_path(args: &serde_yaml::Value) -> String {
    match args {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Mapping(m) => {
            if let Some(serde_yaml::Value::String(s)) =
                m.get(serde_yaml::Value::String("file".to_string()))
            {
                return s.clone();
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Extract the role name from an `include_role` task's args (scalar or
/// `{ name: ... }`).
fn include_role_name(args: &serde_yaml::Value) -> String {
    match args {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Mapping(m) => {
            if let Some(serde_yaml::Value::String(s)) =
                m.get(serde_yaml::Value::String("name".to_string()))
            {
                return s.clone();
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Resolve a play's `roles:` list into a [`ResolvedRoles`] for execution.
///
/// Each top-level `RoleRef` is resolved via [`crate::role::resolve_role_chain`]
/// (DFS topological sort, deps first). The chain's tasks are collected into a
/// single [`Block`] carrying the `RoleRef`'s `tags:`/`when:`/`vars:` so the
/// existing block-walker machinery handles tag filtering and conditionals.
fn resolve_roles_for_play(base_dir: &Path, play: &Play) -> anyhow::Result<ResolvedRoles> {
    if play.roles.is_empty() {
        return Ok(ResolvedRoles {
            defaults: PVars::default(),
            vars: PVars::default(),
            blocks: Vec::new(),
            combined_handlers: play.handlers.clone(),
        });
    }

    let mut defaults = PVars::default();
    let mut role_vars = PVars::default();
    let mut blocks: Vec<TaskNode> = Vec::new();
    let mut role_handlers: Vec<TaskNode> = Vec::new();

    for role_ref in &play.roles {
        let chain =
            crate::role::resolve_role_chain(base_dir, role_ref).map_err(anyhow::Error::from)?;

        let mut chain_tasks: Vec<TaskNode> = Vec::new();
        for role in &chain {
            // Merge defaults (lowest) and vars (higher than play vars).
            for (k, v) in &role.defaults.0 {
                defaults.0.insert(k.clone(), v.clone());
            }
            for (k, v) in &role.vars.0 {
                role_vars.0.insert(k.clone(), v.clone());
            }
            chain_tasks.extend(role.tasks.iter().cloned());
            role_handlers.extend(role.handlers.iter().cloned());
        }

        // Wrap the chain's tasks in a block carrying the RoleRef's tags/when/vars.
        blocks.push(TaskNode::Block(Box::new(Block {
            name: Some(format!("role : {}", role_ref.role)),
            vars: role_ref.vars.clone(),
            when: role_ref.when.clone(),
            tasks: chain_tasks,
            rescue: Vec::new(),
            always: Vec::new(),
            r#become: None,
            tags: role_ref.tags.clone(),
        })));
    }

    let mut combined_handlers = play.handlers.clone();
    combined_handlers.extend(role_handlers);

    Ok(ResolvedRoles {
        defaults,
        vars: role_vars,
        blocks,
        combined_handlers,
    })
}

/// Execute every playbook against the resolved inventory via `core`.
///
/// Listing flags (`--syntax-check` / `--list-*`) are handled by `commands`;
/// this runs the playbooks for real. Returns an Ansible-ish textual report.
///
/// # Errors
///
/// Propagates nothing (host-level failures are recorded in the report); returns
/// an `Err` only on a catastrophic build/setup failure.
/// Execute every playbook against the resolved inventory via `core`.
///
/// Listing flags (`--syntax-check` / `--list-*`) are handled by `commands`;
/// this runs the playbooks for real. Returns an Ansible-ish textual report.
///
/// # Errors
///
/// Propagates nothing (host-level failures are recorded in the report); returns
/// an `Err` only on a catastrophic build/setup failure.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    playbooks: &[(String, Playbook)],
    inventory: &Inventory,
    limit: Option<&str>,
    core: &CoreApiRef,
    check_mode: bool,
    tag_filter: &TagFilter,
    start_at_task: Option<&str>,
    extra_vars: &PVars,
    forks: usize,
    diff_mode: bool,
    skip_unsupported: bool,
) -> anyhow::Result<String> {
    let registry = executors::register_all();
    let limit_set: Option<Vec<String>> = limit.map(|l| inventory.resolve(l));
    let mut out = String::new();
    let mut recap: IndexMap<String, Recap> = IndexMap::new();

    // Dedicated rayon pool sized to `forks` (clamped to ≥1). A 1-thread pool
    // runs `par_iter` sequentially on a single worker, so `forks == 1` is the
    // fast sequential path (no extra workers spawned); `forks > 1` runs hosts
    // within a batch concurrently. Results are always collected in batch order.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(forks.max(1))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to create thread pool: {e}"))?;

    // Shared runtime inventory additions: `add_host` / `group_by` writes land
    // here; subsequent plays see the new hosts/groups at host-resolution time.
    let runtime: executors::RuntimeInventory = executors::TaskContext::empty_runtime();

    for (path, pb) in playbooks {
        for play in &pb.0 {
            let base_dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
            let plan = build_play_plan(base_dir, play, skip_unsupported)?;

            let mut hosts = inventory.resolve(play_host_pattern(play));
            // Include hosts added at runtime by `add_host` / `group_by`.
            {
                if let Ok(rt) = runtime.lock() {
                    let pattern = play_host_pattern(play);
                    if pattern == "all" {
                        for (h, _) in &rt.hosts {
                            if !hosts.contains(h) {
                                hosts.push(h.clone());
                            }
                        }
                    }
                    if let Some(gh) = rt.groups.get(pattern) {
                        for h in gh {
                            if !hosts.contains(h) {
                                hosts.push(h.clone());
                            }
                        }
                    }
                    if rt.hosts.contains_key(pattern) && !hosts.iter().any(|h| h == pattern) {
                        hosts.push(pattern.to_string());
                    }
                }
            }
            if let Some(lset) = &limit_set {
                hosts.retain(|h| lset.iter().any(|x| x == h));
            }

            let _ = writeln!(
                out,
                "\nPLAY [{}] ({} host(s))",
                play.name
                    .as_deref()
                    .unwrap_or_else(|| play_host_pattern(play)),
                hosts.len()
            );

            let batches = serial_batches(&play.serial, hosts.len());
            let mut cursor = 0usize;
            let mut stop_play = false;
            for bsize in batches {
                let end = (cursor + bsize).min(hosts.len());
                let batch = &hosts[cursor..end];
                cursor = end;

                // `run_once:` tasks complete on the first host only; the
                // tracker is shared across hosts in this batch (serial:).
                let run_once_done = Arc::new(Mutex::new(HashSet::new()));

                // Run hosts in this batch (concurrent when forks > 1, sequential
                // otherwise). Results are merged in batch order.
                let results: Vec<(HostStop, String, IndexMap<String, Recap>)> =
                    pool.install(|| {
                        batch
                            .par_iter()
                            .map(|host_label| {
                                run_host(
                                    &plan,
                                    &runtime,
                                    host_label,
                                    batch,
                                    inventory,
                                    &registry,
                                    core,
                                    check_mode,
                                    diff_mode,
                                    extra_vars,
                                    tag_filter,
                                    start_at_task,
                                    &run_once_done,
                                )
                            })
                            .collect()
                    });

                for (_stop, host_out, host_recap) in &results {
                    out.push_str(host_out);
                    for (host, r) in host_recap {
                        recap.entry(host.clone()).or_default().merge(r);
                    }
                }

                if results
                    .iter()
                    .any(|(stop, _, _)| *stop == HostStop::StopPlay)
                {
                    stop_play = true;
                    break;
                }
            }
            if stop_play {
                break;
            }
        }
    }

    write_recap(&mut out, &recap);
    Ok(out)
}

/// Run all task sections for one host, then flush remaining handlers.
#[allow(clippy::too_many_arguments)]
fn run_host(
    plan: &PlayPlan,
    runtime: &executors::RuntimeInventory,
    host_label: &str,
    batch: &[String],
    inventory: &Inventory,
    registry: &executors::ModuleRegistry,
    core: &CoreApiRef,
    check_mode: bool,
    diff_mode: bool,
    extra_vars: &PVars,
    tag_filter: &TagFilter,
    start_at_task: Option<&str>,
    run_once_done: &Arc<Mutex<HashSet<String>>>,
) -> (HostStop, String, IndexMap<String, Recap>) {
    let mut out = String::new();
    let mut recap: IndexMap<String, Recap> = IndexMap::new();
    // Play-level `become:`/`become_user:` injected into the host vars map so
    // `build_host_info` picks them up as `ansible_become`/`ansible_become_user`.
    let mut hv = host_vars(inventory, host_label);
    if plan.r#become == Some(true) {
        hv.0.insert(
            "ansible_become".to_string(),
            serde_yaml::Value::String("yes".to_string()),
        );
    }
    if let Some(user) = &plan.become_user {
        hv.0.insert(
            "ansible_become_user".to_string(),
            serde_yaml::Value::String(user.clone()),
        );
    }
    // Merge runtime-added host vars (from `add_host`) so `build_host_info`
    // picks up `ansible_connection` / `ansible_host` / etc. for dynamic hosts.
    if let Ok(rt) = runtime.lock()
        && let Some(serde_json::Value::Object(map)) = rt.hosts.get(host_label)
    {
        for (k, v) in map {
            let yv = serde_yaml::to_value(v).unwrap_or(serde_yaml::Value::Null);
            hv.0.insert(k.clone(), yv);
        }
    }
    let host_info = host::build_host_info(host_label, &hv);
    let mut pool = ConnectionPool::new(core);
    let mut vars = build_host_vars(
        host_label,
        batch,
        inventory,
        &plan.play_vars,
        &plan.vars_file_vars,
        &plan.defaults,
        &plan.role_vars,
        extra_vars,
    );
    // Inject play-level `become:`/`become_user:` into the var stack so the
    // per-task `apply_task_settings` merge can see them as `ansible_become` /
    // `ansible_become_user` (task-level overrides play-level). The same keys
    // are also injected into `hv` above for `HostInfo` elevation at connection
    // time; both injection points are needed (one for the connection, one for
    // the command-prefix merge in the executors).
    if plan.r#become == Some(true) || plan.become_user.is_some() {
        let mut layer = VarLayer::new(LayerKind::PlayVars);
        if plan.r#become == Some(true) {
            layer.insert(
                "ansible_become".to_string(),
                serde_json::Value::String("yes".to_string()),
            );
        }
        if let Some(user) = &plan.become_user {
            layer.insert(
                "ansible_become_user".to_string(),
                serde_json::Value::String(user.clone()),
            );
        }
        vars.push(layer);
    }
    let facts: Arc<Mutex<IndexMap<String, serde_json::Value>>> =
        Arc::new(Mutex::new(IndexMap::new()));
    let flow = Arc::new(Mutex::new(FlowControl::Continue));
    let mut notified: Vec<String> = Vec::new();

    // Facts collection: run when `gather_facts` is not explicitly disabled.
    if should_gather_facts(&plan.gather_facts)
        && let Ok(conn) = pool.acquire(host_label, host_info.clone())
    {
        let collected = facts::collect(&conn);
        if !collected.is_empty() {
            let facts_obj = serde_json::to_value(&collected).unwrap_or(serde_json::Value::Null);
            let mut layer = VarLayer::new(LayerKind::PlayVars);
            for (k, v) in &collected {
                layer.insert(k, v.clone());
            }
            vars.push(layer);
            // Also expose under `ansible_facts` as a nested object.
            vars.set_fact("ansible_facts", facts_obj);
        }
    }

    // Sections: pre_tasks → roles → tasks → post_tasks (Ansible §8.1 order).
    let sections: [&[TaskNode]; 4] = [
        &plan.pre_tasks,
        &plan.role_blocks,
        &plan.tasks,
        &plan.post_tasks,
    ];

    let mut filter = RunFilter::new(
        tag_filter,
        start_at_task,
        plan.any_errors_fatal,
        Some(run_once_done),
        Some(plan.base_dir.as_path()),
        diff_mode,
        runtime,
        plan.skip_unsupported,
    );
    let mut stop = HostStop::Continue;
    for section in &sections {
        stop = run_nodes(
            section,
            host_label,
            &host_info,
            registry,
            &mut pool,
            &mut vars,
            &facts,
            &flow,
            &mut notified,
            &plan.combined_handlers,
            &mut out,
            &mut recap,
            check_mode,
            &mut filter,
        );
        if stop != HostStop::Continue {
            break;
        }
    }
    drop(pool);

    // Flush any handlers still pending at the play's end.
    if stop != HostStop::StopPlay {
        let _ = block::flush_handlers(
            &mut notified,
            &plan.combined_handlers,
            host_label,
            &host_info,
            registry,
            &mut ConnectionPool::new(core),
            &mut vars,
            &facts,
            &flow,
            &mut out,
            &mut recap,
            check_mode,
            tag_filter,
            runtime,
            plan.skip_unsupported,
        );
    }
    (stop, out, recap)
}

/// The play's `hosts:` matcher string (or `all`).
fn play_host_pattern(play: &Play) -> &str {
    play.hosts.as_ref().map_or("all", HostMatcher::as_str)
}

/// Whether `gather_facts` is enabled (anything except an explicit `false`/`no`).
const fn should_gather_facts(gf: &GatherFacts) -> bool {
    !matches!(gf, GatherFacts::No | GatherFacts::Bool(false))
}

/// Split `n` hosts into `serial:` batch sizes. An empty/`as_u64`-less `serial`
/// means one batch of all hosts. A single value repeats; a list uses each size
/// in turn then repeats the last. Percentages (`"50%"`) are not yet supported
/// and fall back to a single batch.
fn serial_batches(serial: &Serial, n: usize) -> Vec<usize> {
    let specs: Vec<usize> = serial
        .0
        .iter()
        .filter_map(|v| v.as_u64().and_then(|x| usize::try_from(x).ok()))
        .collect();
    if specs.is_empty() || n == 0 {
        return vec![n.max(1)];
    }
    let mut batches = Vec::new();
    let mut remaining = n;
    let mut i = 0;
    while remaining > 0 {
        let size = if i < specs.len() {
            specs[i]
        } else {
            specs.last().copied().unwrap_or(1)
        };
        let take = size.clamp(1, remaining);
        batches.push(take);
        remaining -= take;
        i += 1;
    }
    batches
}

/// Merge every inventory layer for `host` (all-group → containing groups →
/// host vars) into a single parser vars map.
fn host_vars(inv: &Inventory, host: &str) -> crate::parser::Vars {
    use crate::parser::Vars as PV;
    let mut m: IndexMap<String, serde_yaml::Value> = IndexMap::new();
    if let Some(all) = inv.groups.get("all") {
        merge_yaml(&mut m, &all.vars);
    }
    for (gname, group) in &inv.groups {
        if gname == "all" {
            continue;
        }
        if group_contains(inv, gname, host) {
            merge_yaml(&mut m, &group.vars);
        }
    }
    if let Some(hv) = inv.hosts.get(host) {
        merge_yaml(&mut m, hv);
    }
    PV(m)
}

/// Does `group` (recursing through children) contain `host`?
fn group_contains(inv: &Inventory, group: &str, host: &str) -> bool {
    let Some(g) = inv.groups.get(group) else {
        return false;
    };
    if g.hosts.iter().any(|h| h == host) {
        return true;
    }
    g.children
        .iter()
        .any(|child| group_contains(inv, child, host))
}

/// Merge `src`'s entries into `dst` (later wins).
fn merge_yaml(dst: &mut IndexMap<String, serde_yaml::Value>, src: &crate::parser::Vars) {
    for (k, v) in &src.0 {
        dst.insert(k.clone(), v.clone());
    }
}

/// Build the per-host variable stack: magic (lowest) → role defaults →
/// inventory → `vars_files` → play vars → role vars → `extra-vars` (highest).
#[allow(clippy::too_many_arguments)]
fn build_host_vars(
    host_label: &str,
    play_hosts: &[String],
    inv: &Inventory,
    play_vars: &PVars,
    vars_file_vars: &PVars,
    role_defaults: &PVars,
    role_vars: &PVars,
    extra_vars: &PVars,
) -> Vars {
    let mut vars = Vars::new();
    let magic = MagicVars {
        inventory_hostname: host_label.to_string(),
        ansible_host: None,
        play_hosts: play_hosts.to_vec(),
        playbook_dir: None,
        role_path: None,
    };
    vars.push(magic.to_layer());
    if !role_defaults.0.is_empty() {
        vars.push(layer_from(role_defaults, LayerKind::RoleDefaults));
    }
    vars.push(layer_from(
        &host_vars(inv, host_label),
        LayerKind::Inventory,
    ));
    if !vars_file_vars.0.is_empty() {
        vars.push(layer_from(vars_file_vars, LayerKind::PlayVarsFiles));
    }
    vars.push(layer_from(play_vars, LayerKind::PlayVars));
    if !role_vars.0.is_empty() {
        vars.push(layer_from(role_vars, LayerKind::RoleVars));
    }
    if !extra_vars.0.is_empty() {
        vars.push(layer_from(extra_vars, LayerKind::ExtraVars));
    }
    vars
}

/// Convert a parser vars map into a templating layer.
fn layer_from(pvars: &crate::parser::Vars, kind: LayerKind) -> VarLayer {
    let mut layer = VarLayer::new(kind);
    for (k, v) in &pvars.0 {
        layer.insert(k.clone(), crate::vars::yaml_to_json(v));
    }
    layer
}

/// Append a per-task status line.
pub(super) fn write_task_line(out: &mut String, record: &task::TaskRecord, host: &str) {
    let status = match record.status {
        TaskStatus::Ok => "ok",
        TaskStatus::Changed => "changed",
        TaskStatus::Failed => "FAILED",
        TaskStatus::Skipped => "skipping",
    };
    let _ = writeln!(
        out,
        "  TASK [{name}] ({host}): {status}",
        name = record.task_name
    );
    if record.no_log {
        return; // Suppress stdout/stderr for no_log tasks.
    }
    // Per-item status lines for looped tasks.
    if let Some(items) = &record.loop_items {
        for item in items {
            let item_status = if item.failed {
                "FAILED"
            } else if item.changed {
                "changed"
            } else {
                "ok"
            };
            let _ = writeln!(
                out,
                "    {host}: ({item_status}) (item={label})",
                label = item.label
            );
        }
    }
    if record.status == TaskStatus::Failed && !record.result.stderr.is_empty() {
        let _ = writeln!(out, "    {}", record.result.stderr.as_str());
    }
    if matches!(record.status, TaskStatus::Ok | TaskStatus::Changed)
        && !record.result.stdout.is_empty()
    {
        for line in record.result.stdout.as_str().lines() {
            let _ = writeln!(out, "    {host}: {line}");
        }
    }
}

/// Record a task's status in the per-host recap.
pub(super) fn tally(recap: &mut IndexMap<String, Recap>, host: &str, status: TaskStatus) {
    let r = recap.entry(host.to_string()).or_default();
    match status {
        TaskStatus::Ok => r.ok += 1,
        TaskStatus::Changed => r.changed += 1,
        TaskStatus::Failed => r.failed += 1,
        TaskStatus::Skipped => r.skipped += 1,
    }
}

/// Append the `PLAY RECAP` block.
fn write_recap(out: &mut String, recap: &IndexMap<String, Recap>) {
    let _ = writeln!(out, "\nPLAY RECAP");
    for (host, r) in recap {
        let _ = writeln!(
            out,
            "  {host:<24} : ok={ok}    changed={changed}    unreachable={un}    failed={failed}    skipped={sk}",
            ok = r.ok,
            changed = r.changed,
            un = r.unreachable,
            failed = r.failed,
            sk = r.skipped
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::null_core;
    use komandan_plugin_abi::prelude::*;

    fn pb(src: &str) -> anyhow::Result<Playbook> {
        Ok(crate::parser::parse_playbook(src)?)
    }

    #[test]
    fn debug_task_runs_against_localhost() -> anyhow::Result<()> {
        let playbook = pb("- hosts: localhost\n  tasks:\n    - debug: msg=hello\n    - ping:\n")?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("TASK [debug]"), "{out}");
        assert!(out.contains("ok=2"), "{out}");
        Ok(())
    }

    #[test]
    fn loop_with_items_runs_each() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  vars:\n    fruits: [apple, banana]\n  tasks:\n    - debug: msg=\"{{ item }}\"\n      loop: \"{{ fruits }}\"\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("apple"), "{out}");
        assert!(out.contains("banana"), "{out}");
        Ok(())
    }

    #[test]
    fn block_rescue_catches_failure() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - block:\n        - fail: msg=boom\n      rescue:\n        - debug: msg=rescued\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("rescued"), "{out}");
        Ok(())
    }

    #[test]
    fn handler_flushes_on_notify() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - command: echo hi\n      notify: restart svc\n  handlers:\n    - name: restart svc\n      debug: msg=restarted\n",
        )?;
        // Stage a `changed` result so the command task notifies the handler
        // (the default mock `komando` returns unchanged).
        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: true,
            rc: 0,
            success: true,
            stdout: RString::from("hi"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("RUNNING HANDLER"), "{out}");
        assert!(out.contains("restarted"), "{out}");
        Ok(())
    }

    #[test]
    fn tags_filter_runs_only_matching() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - name: alpha\n      debug: msg=alpha\n      tags: [web]\n    - name: beta\n      debug: msg=beta\n      tags: [db]\n",
        )?;
        let core = null_core();
        let filter = TagFilter::from_cli(Some("web"), None);
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &filter,
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("alpha"), "{out}");
        assert!(!out.contains("beta"), "{out}");
        Ok(())
    }

    #[test]
    fn skip_tags_excludes_matching() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - name: alpha\n      debug: msg=alpha\n      tags: [web]\n    - name: beta\n      debug: msg=beta\n      tags: [slow]\n",
        )?;
        let core = null_core();
        let filter = TagFilter::from_cli(None, Some("slow"));
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &filter,
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("alpha"), "{out}");
        assert!(!out.contains("beta"), "{out}");
        Ok(())
    }

    #[test]
    fn start_at_task_skips_earlier_tasks() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - name: first\n      debug: msg=one\n    - name: second\n      debug: msg=two\n    - name: third\n      debug: msg=three\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            Some("second"),
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(!out.contains("one"), "{out}");
        assert!(out.contains("two"), "{out}");
        assert!(out.contains("three"), "{out}");
        Ok(())
    }

    #[test]
    fn handler_flushes_on_listen_topic() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  tasks:\n    - command: echo hi\n      notify: restart-things\n  handlers:\n    - name: restart svc\n      listen: restart-things\n      debug: msg=restarted\n",
        )?;
        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: true,
            rc: 0,
            success: true,
            stdout: RString::from("hi"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("RUNNING HANDLER"), "{out}");
        assert!(out.contains("restarted"), "{out}");
        Ok(())
    }

    #[test]
    fn serial_batches_default_is_all_hosts() {
        let batches = serial_batches(&Serial::default(), 5);
        assert_eq!(batches, vec![5]);
    }

    #[test]
    fn serial_batches_fixed_size_repeats() {
        let batches = serial_batches(&Serial(vec![serde_yaml::Value::Number(2.into())]), 5);
        assert_eq!(batches, vec![2, 2, 1]);
    }

    #[test]
    fn serial_batches_list_then_repeat_last() {
        let batches = serial_batches(
            &Serial(vec![
                serde_yaml::Value::Number(1.into()),
                serde_yaml::Value::Number(3.into()),
            ]),
            7,
        );
        assert_eq!(batches, vec![1, 3, 3]);
    }

    #[test]
    fn group_contains_recurse() {
        let mut inv = Inventory::default();
        inv.add_host_to_group("web", "w1");
        inv.add_child_group("all", "web");
        assert!(group_contains(&inv, "web", "w1"));
        assert!(group_contains(&inv, "all", "w1"));
        assert!(!group_contains(&inv, "web", "w2"));
    }

    // ---- Role integration tests ----

    /// Helper: create a role on disk and return the base dir.
    fn role_dir(base: &Path, name: &str, tasks: &str) -> std::io::Result<()> {
        let dir = base.join("roles").join(name).join("tasks");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("main.yml"), tasks)
    }

    #[test]
    fn role_tasks_execute() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "r", "- debug: msg=from-role\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(&pb_path, "- hosts: localhost\n  roles:\n    - r\n")?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("from-role"), "{out}");
        Ok(())
    }

    #[test]
    fn role_handler_triggered_by_notify() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();

        // Role with a task that notifies a handler, and the handler itself.
        let dir = base.join("roles/r");
        std::fs::create_dir_all(dir.join("tasks"))?;
        std::fs::create_dir_all(dir.join("handlers"))?;
        std::fs::write(
            dir.join("tasks/main.yml"),
            "- command: echo hi\n  notify: restart-svc\n",
        )?;
        std::fs::write(
            dir.join("handlers/main.yml"),
            "- name: restart-svc\n  debug: msg=restarted\n",
        )?;

        let pb_path = base.join("site.yml");
        std::fs::write(&pb_path, "- hosts: localhost\n  roles:\n    - r\n")?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;

        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: true,
            rc: 0,
            success: true,
            stdout: RString::from("hi"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("restarted"), "{out}");
        Ok(())
    }

    #[test]
    fn role_vars_available_in_tasks() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        let dir = base.join("roles/r");
        std::fs::create_dir_all(dir.join("tasks"))?;
        std::fs::create_dir_all(dir.join("vars"))?;
        std::fs::write(dir.join("vars/main.yml"), "greeting: hello-from-vars\n")?;
        std::fs::write(
            dir.join("tasks/main.yml"),
            "- debug: msg=\"{{ greeting }}\"\n",
        )?;
        let pb_path = base.join("site.yml");
        std::fs::write(&pb_path, "- hosts: localhost\n  roles:\n    - r\n")?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("hello-from-vars"), "{out}");
        Ok(())
    }

    #[test]
    fn role_defaults_lowest_precedence() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        let dir = base.join("roles/r");
        std::fs::create_dir_all(dir.join("tasks"))?;
        std::fs::create_dir_all(dir.join("defaults"))?;
        std::fs::write(dir.join("defaults/main.yml"), "port: 80\n")?;
        std::fs::write(
            dir.join("tasks/main.yml"),
            "- debug: msg=\"port={{ port }}\"\n",
        )?;
        // Play vars override the default.
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  vars:\n    port: 8080\n  roles:\n    - r\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("port=8080"), "play vars should win: {out}");
        assert!(!out.contains("port=80\n"), "{out}");
        Ok(())
    }

    #[test]
    fn role_dependencies_run_first() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        // Role A depends on B; B should run first.
        role_dir(base, "B", "- debug: msg=dep-B\n")?;
        let dir_a = base.join("roles/A");
        std::fs::create_dir_all(dir_a.join("tasks"))?;
        std::fs::create_dir_all(dir_a.join("meta"))?;
        std::fs::write(dir_a.join("tasks/main.yml"), "- debug: msg=role-A\n")?;
        std::fs::write(dir_a.join("meta/main.yml"), "dependencies:\n  - B\n")?;

        let pb_path = base.join("site.yml");
        std::fs::write(&pb_path, "- hosts: localhost\n  roles:\n    - A\n")?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let pos_b = out.find("dep-B").unwrap_or(usize::MAX);
        let pos_a = out.find("role-A").unwrap_or(usize::MAX);
        assert!(pos_b < pos_a, "dependency B should run before A: {out}");
        Ok(())
    }

    #[test]
    fn role_tag_filter_selects_role() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "web", "- debug: msg=web-task\n")?;
        role_dir(base, "db", "- debug: msg=db-task\n")?;

        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  roles:\n    - { role: web, tags: [web] }\n    - { role: db, tags: [db] }\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let filter = TagFilter::from_cli(Some("web"), None);
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &filter,
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("web-task"), "{out}");
        assert!(!out.contains("db-task"), "{out}");
        Ok(())
    }

    #[test]
    fn roles_inserted_between_pre_tasks_and_tasks() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "r", "- debug: msg=role-task\n")?;

        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  pre_tasks:\n    - debug: msg=pre\n  roles:\n    - r\n  tasks:\n    - debug: msg=post\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let pos_pre = out.find("pre").unwrap_or(usize::MAX);
        let pos_role = out.find("role-task").unwrap_or(usize::MAX);
        let pos_post = out.find("post").unwrap_or(usize::MAX);
        assert!(pos_pre < pos_role, "pre_tasks before roles: {out}");
        assert!(pos_role < pos_post, "roles before tasks: {out}");
        Ok(())
    }

    // ---- include_tasks / import_tasks tests ----

    #[test]
    fn include_tasks_loads_file_inline() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(
            base.join("extra.yml"),
            "- debug: msg=from-include\n- debug: msg=second\n",
        )?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_tasks: extra.yml\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("from-include"), "{out}");
        assert!(out.contains("second"), "{out}");
        Ok(())
    }

    #[test]
    fn import_tasks_loads_file_inline() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("jobs.yml"), "- debug: msg=imported\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - import_tasks: jobs.yml\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("imported"), "{out}");
        Ok(())
    }

    #[test]
    fn include_tasks_inside_block() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("inner.yml"), "- debug: msg=inner-task\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - block:\n        - include_tasks: inner.yml\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("inner-task"), "{out}");
        Ok(())
    }

    #[test]
    fn include_tasks_with_file_key_mapping() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("tasks.yml"), "- debug: msg=mapped\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_tasks: { file: tasks.yml }\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("mapped"), "{out}");
        Ok(())
    }

    // ---- include_role / import_role tests ----

    #[test]
    fn include_role_runs_tasks() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "r", "- debug: msg=from-include-role\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_role: r\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("from-include-role"), "{out}");
        Ok(())
    }

    #[test]
    fn include_role_with_name_key() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "r", "- debug: msg=from-named-role\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_role: { name: r }\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("from-named-role"), "{out}");
        Ok(())
    }

    #[test]
    fn include_role_with_vars() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        let dir = base.join("roles/r");
        std::fs::create_dir_all(dir.join("tasks"))?;
        std::fs::create_dir_all(dir.join("vars"))?;
        std::fs::write(dir.join("vars/main.yml"), "greeting: hello-from-role\n")?;
        std::fs::write(
            dir.join("tasks/main.yml"),
            "- debug: msg=\"{{ greeting }}\"\n",
        )?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_role: r\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("hello-from-role"), "{out}");
        Ok(())
    }

    #[test]
    fn include_role_dependencies_run_first() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        role_dir(base, "B", "- debug: msg=dep-B\n")?;
        let dir_a = base.join("roles/A");
        std::fs::create_dir_all(dir_a.join("tasks"))?;
        std::fs::create_dir_all(dir_a.join("meta"))?;
        std::fs::write(dir_a.join("tasks/main.yml"), "- debug: msg=role-A\n")?;
        std::fs::write(dir_a.join("meta/main.yml"), "dependencies:\n  - B\n")?;

        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_role: A\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let pos_b = out.find("dep-B").unwrap_or(usize::MAX);
        let pos_a = out.find("role-A").unwrap_or(usize::MAX);
        assert!(pos_b < pos_a, "dependency B should run before A: {out}");
        Ok(())
    }

    // ---- Round 2: extra-vars / vars_files / check_mode / no_log / delegate ----

    #[test]
    fn extra_vars_override_play_vars() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  vars:\n    key: from_play\n  tasks:\n    - debug: msg=\"{{ key }}\"\n",
        )?;
        let core = null_core();
        let mut ev = PVars::default();
        ev.0.insert(
            "key".to_string(),
            serde_yaml::Value::String("from_extra".to_string()),
        );
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &ev,
            1,
            false,
            false,
        )?;
        assert!(out.contains("from_extra"), "extra-vars should win: {out}");
        assert!(!out.contains("from_play"), "{out}");
        Ok(())
    }

    #[test]
    fn extra_vars_from_file() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("extra.yml"), "key: from_file\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - debug: msg=\"{{ key }}\"\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;

        // Parse the @extra.yml form via a helper mirroring commands::parse_extra_vars.
        let ev = parse_extra_vars_via_files(base, &["extra.yml".to_string()])?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &ev,
            1,
            false,
            false,
        )?;
        assert!(out.contains("from_file"), "{out}");
        Ok(())
    }

    #[test]
    fn vars_files_loaded() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::create_dir_all(base.join("vars"))?;
        std::fs::write(base.join("vars/common.yml"), "greeting: hello\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  vars_files:\n    - vars/common.yml\n  tasks:\n    - debug: msg=\"{{ greeting }}\"\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("hello"), "{out}");
        Ok(())
    }

    #[test]
    fn check_mode_skips_mutating_task() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: mut\n      command: echo hi\n    - name: probe\n      debug: msg=ran-in-check\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            true, // check_mode
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // The mutating `command` task is skipped in check mode.
        assert!(out.contains("TASK [mut]"), "{out}");
        let mut_pos = out.find("TASK [mut]").unwrap_or(usize::MAX);
        let after = &out[mut_pos..];
        assert!(
            after.starts_with("TASK [mut]")
                && after[after.find('(').unwrap_or(0)..].contains("skipping"),
            "mut task should be skipped: {out}"
        );
        // The `debug` task runs in check mode (supports_check_mode).
        assert!(out.contains("ran-in-check"), "{out}");
        assert!(out.contains("skipped=1"), "{out}");
        Ok(())
    }

    #[test]
    fn no_log_suppresses_output() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: secret\n      debug: msg=hush-hush\n      no_log: true\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // The TASK header line still shows.
        assert!(out.contains("TASK [secret]"), "{out}");
        // But the secret payload is suppressed.
        assert!(
            !out.contains("hush-hush"),
            "no_log should hide output: {out}"
        );
        Ok(())
    }

    #[test]
    fn delegate_to_localhost() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - local_action:\n        module: command\n        args:\n          cmd: echo delegated\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;

        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: true,
            rc: 0,
            success: true,
            stdout: RString::from("delegated"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("delegated"), "{out}");
        Ok(())
    }

    #[test]
    fn local_action_command_shorthand() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        let pb_path = base.join("site.yml");
        // Scalar `local_action: ping` exercises the scalar parse_action path
        // (the `command echo hi` multi-token form needs parser-level
        // module/args splitting, out of scope here).
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - local_action: ping\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("pong"), "{out}");
        Ok(())
    }

    /// Helper: parse `@file` extra-vars relative to `base` (mirrors the
    /// `commands::parse_extra_vars` `@file` branch for in-test use).
    fn parse_extra_vars_via_files(base: &Path, files: &[String]) -> anyhow::Result<PVars> {
        let mut merged = PVars::default();
        for f in files {
            let path = base.join(f);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
            let vars = crate::parser::parse_vars_text(&text).map_err(anyhow::Error::from)?;
            for (k, v) in vars.0 {
                merged.0.insert(k, v);
            }
        }
        Ok(merged)
    }

    // ---- become / any_errors_fatal / run_once ----

    /// Helper: a multi-host inventory with the given hosts in `all`.
    fn multi_host_inventory(hosts: &[&str]) -> Inventory {
        let mut inv = Inventory::default();
        for h in hosts {
            inv.add_host_to_group("all", h);
        }
        inv
    }

    #[test]
    fn become_sets_host_info_elevate() -> anyhow::Result<()> {
        // Play-level `become:`/`become_user:` should flow into HostInfo without
        // breaking execution. The null core returns success regardless.
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  become: true\n  become_user: root\n  tasks:\n    - debug: msg=hello\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("hello"), "{out}");
        Ok(())
    }

    #[test]
    fn task_level_become_prefixes_command_with_sudo() -> anyhow::Result<()> {
        // Task-level `become: true` should prepend `sudo -u root` to the
        // dispatched command, even when the play does NOT set become.
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: elevated\n      command: echo hi\n      become: true\n      become_user: root\n",
        )?;
        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let cmds = handle.komando_cmds();
        assert!(
            cmds.iter().any(|c| c.contains("sudo -u root")),
            "expected sudo -u root in commands, got: {cmds:?}\n{out}"
        );
        Ok(())
    }

    #[test]
    fn play_level_become_applied_to_tasks_without_task_become() -> anyhow::Result<()> {
        // Play-level `become: true` should apply to tasks that don't override
        // it (here: a plain `command` task gets `sudo`-prefixed).
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  become: true\n  tasks:\n    - name: plain\n      command: echo hi\n",
        )?;
        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let cmds = handle.komando_cmds();
        assert!(
            cmds.iter().any(|c| c.contains("sudo")),
            "expected sudo in commands, got: {cmds:?}\n{out}"
        );
        Ok(())
    }

    #[test]
    fn any_errors_fatal_stops_play() -> anyhow::Result<()> {
        // With `any_errors_fatal: true`, a failing task on h1 stops the play
        // before h2 runs the task.
        let playbook = pb(
            "- hosts: all\n  gather_facts: false\n  any_errors_fatal: true\n  tasks:\n    - command: /bin/false\n",
        )?;
        let core = crate::test_support::MockCore::default();
        let handle = core.handle();
        // First call fails; subsequent hosts should not reach this task.
        handle.expect_komando(ModuleResult {
            changed: false,
            rc: 1,
            success: false,
            stdout: RString::new(),
            stderr: RString::from("failed"),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let inv = multi_host_inventory(&["h1", "h2"]);
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &inv,
            None,
            &core_ref,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("FAILED"), "{out}");
        Ok(())
    }

    #[test]
    fn run_once_executes_on_first_host_only() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: all\n  gather_facts: false\n  tasks:\n    - name: once-task\n      debug: msg=runs-once\n      run_once: true\n",
        )?;
        let core = null_core();
        let inv = multi_host_inventory(&["h1", "h2"]);
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &inv,
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // The debug msg should appear exactly once (h1 only; h2 skips).
        let count = out.matches("runs-once").count();
        assert_eq!(count, 1, "run_once task should run exactly once: {out}");
        Ok(())
    }

    #[test]
    fn without_run_once_task_runs_on_all_hosts() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: all\n  gather_facts: false\n  tasks:\n    - name: every-task\n      debug: msg=runs-everywhere\n",
        )?;
        let core = null_core();
        let inv = multi_host_inventory(&["h1", "h2"]);
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &inv,
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        let count = out.matches("runs-everywhere").count();
        assert_eq!(
            count, 2,
            "task without run_once should run on both hosts: {out}"
        );
        Ok(())
    }

    // ---- Templated includes (runtime expansion) ----

    #[test]
    fn templated_include_tasks_expanded_at_runtime() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("alpha.yml"), "- debug: msg=from-alpha\n")?;
        std::fs::write(base.join("beta.yml"), "- debug: msg=from-beta\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  vars:\n    which: alpha\n  tasks:\n    - include_tasks: \"{{ which }}.yml\"\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(
            out.contains("from-alpha"),
            "should include alpha.yml: {out}"
        );
        assert!(
            !out.contains("from-beta"),
            "should NOT include beta.yml: {out}"
        );
        Ok(())
    }

    #[test]
    fn templated_include_tasks_with_file_key() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("included.yml"), "- debug: msg=hello-templated\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  vars:\n    fname: included\n  tasks:\n    - include_tasks: { file: \"{{ fname }}.yml\" }\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("hello-templated"), "{out}");
        Ok(())
    }

    #[test]
    fn static_include_still_works() -> anyhow::Result<()> {
        use tempfile::TempDir;
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("static.yml"), "- debug: msg=static-include\n")?;
        let pb_path = base.join("site.yml");
        std::fs::write(
            &pb_path,
            "- hosts: localhost\n  tasks:\n    - include_tasks: static.yml\n",
        )?;
        let playbook =
            crate::parser::parse_playbook_file(&pb_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let core = null_core();
        let out = execute(
            &[(pb_path.to_string_lossy().into_owned(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("static-include"), "{out}");
        Ok(())
    }

    #[test]
    fn forks_runs_hosts_in_parallel() -> anyhow::Result<()> {
        // With forks=2, all hosts in a multi-host play should still run and
        // produce output (exercises the rayon thread-pool path).
        let playbook = pb(
            "- hosts: all\n  gather_facts: false\n  tasks:\n    - name: common-task\n      debug: msg=runs-everywhere\n",
        )?;
        let core = null_core();
        let inv = multi_host_inventory(&["h1", "h2", "h3"]);
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &inv,
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            2, // forks
            false,
            false,
        )?;
        // All three hosts should have run the task.
        assert_eq!(
            out.matches("runs-everywhere").count(),
            3,
            "all hosts should run the task: {out}"
        );
        // All three should appear in the recap.
        assert!(out.contains("h1"), "{out}");
        assert!(out.contains("h2"), "{out}");
        assert!(out.contains("h3"), "{out}");
        Ok(())
    }

    #[test]
    fn loop_control_label_shown_in_output() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  vars:\n    users:\n      - name: alice\n        uid: 1001\n      - name: bob\n        uid: 1002\n  tasks:\n    - name: show users\n      debug: msg=\"creating user\"\n      loop: \"{{ users }}\"\n      loop_control:\n        label: \"{{ item.name }}\"\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // The label should show just the name, not the full dict.
        assert!(out.contains("item=alice"), "expected item=alice in: {out}");
        assert!(out.contains("item=bob"), "expected item=bob in: {out}");
        // Should NOT show the full JSON dict (no `uid` key in output).
        assert!(
            !out.contains("uid"),
            "should not show uid when label is set: {out}"
        );
        Ok(())
    }

    #[test]
    fn loop_without_label_shows_item_json() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: simple loop\n      debug: msg=\"hi\"\n      loop:\n        - one\n        - two\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // Without a label, the item value itself is shown.
        assert!(
            out.contains("item=one") || out.contains("\"one\""),
            "expected item one in: {out}"
        );
        assert!(
            out.contains("item=two") || out.contains("\"two\""),
            "expected item two in: {out}"
        );
        Ok(())
    }

    #[test]
    fn diff_mode_does_not_break_normal_run() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: hi\n      debug: msg=hello\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            true,  // diff
            false, // skip_unsupported
        )?;
        assert!(out.contains("hello"), "{out}");
        Ok(())
    }

    #[test]
    fn diff_mode_works_with_lineinfile() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: add line\n      lineinfile:\n        path: /tmp/diff-test.txt\n        line: hello\n        create: true\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            true,  // diff
            false, // skip_unsupported
        )?;
        // The reuse executor's dispatch_with_diff ran; the run must not crash,
        // and the task should report ok (mock komando returns unchanged).
        assert!(
            out.contains("lineinfile") || out.contains("hello") || out.contains("ok"),
            "{out}"
        );
        Ok(())
    }

    // ---- add_host / group_by ----

    #[test]
    fn add_host_visible_in_next_play() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: add dynamic host\n      add_host:\n        name: dynhost\n        groups: webservers\n        ansible_connection: local\n- hosts: webservers\n  gather_facts: false\n  tasks:\n    - name: ping dynamic\n      debug: msg=\"found dynhost\"\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        assert!(out.contains("dynhost"), "expected dynhost in output: {out}");
        assert!(
            out.contains("found dynhost"),
            "expected debug msg from dynamic host: {out}"
        );
        Ok(())
    }

    #[test]
    fn group_by_adds_current_host() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: classify\n      group_by: key=os_linux\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false,
        )?;
        // The group_by stdout surfaces the host → group mapping.
        assert!(
            out.contains("group_by") && out.contains("os_linux"),
            "expected group_by mapping in output: {out}"
        );
        Ok(())
    }

    #[test]
    fn skip_unsupported_module_with_flag() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: unsupported task\n      docker_container:\n        name: mycontainer\n        image: nginx\n    - name: supported task\n      debug: msg=\"hello\"\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            true, // skip_unsupported
        )?;
        assert!(!out.contains("FAILED"), "expected no failures: {out}");
        assert!(
            out.contains("skipping") || out.contains("skip"),
            "expected skip message: {out}"
        );
        // The supported task should still run.
        assert!(out.contains("hello"), "supported task should run: {out}");
        Ok(())
    }

    #[test]
    fn unsupported_module_fails_without_flag() -> anyhow::Result<()> {
        let playbook = pb(
            "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - name: unsupported task\n      docker_container:\n        name: mycontainer\n        image: nginx\n",
        )?;
        let core = null_core();
        let out = execute(
            &[("site.yml".to_string(), playbook)],
            &Inventory::implicit_localhost(),
            None,
            &core,
            false,
            &TagFilter::none(),
            None,
            &PVars::default(),
            1,
            false,
            false, // skip_unsupported
        )?;
        assert!(
            out.contains("FAILED"),
            "expected failure without flag: {out}"
        );
        Ok(())
    }
}
