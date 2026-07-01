//! Playbook intermediate representation.
//!
//! Plain data structs produced by [`super::yaml`] from the raw YAML tree.
//! Kept free of `serde` derives on purpose: the Ansible "unknown top-level
//! task key ⇒ module name + args" rule (see [`super::yaml::parse_task`]) does
//! not map cleanly onto derive, so the parser walks the YAML values explicitly
//! and constructs these structs. Fields unused by the Phase 1 listing commands
//! (`serial`, `gather_facts` detail) are kept as raw [`serde_yaml::Value`] to
//! stay lenient; later phases refine them.
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §4.

use std::collections::HashMap;

use indexmap::IndexMap;

/// A parsed playbook — an ordered list of plays (a YAML file is a sequence of
/// play mappings).
#[derive(Debug, Clone, Default)]
pub struct Playbook(pub Vec<Play>);

/// A single play: a host matcher plus its task lists.
#[derive(Debug, Clone, Default)]
pub struct Play {
    /// `import_playbook: file.yml` — expanded in-place by `parse_playbook_file`.
    /// After expansion, this is always `None` (the directive is replaced by the
    /// imported plays).
    pub import_playbook: Option<String>,
    /// `hosts:` matcher (a group name, `all`, `*`, or comma list). Resolved
    /// against inventory at list/run time. Required by Ansible; the parser
    /// surfaces a structural error when absent.
    pub hosts: Option<HostMatcher>,
    pub name: Option<String>,
    pub vars: Vars,
    pub r#become: Option<bool>,
    pub become_user: Option<String>,
    pub remote_user: Option<String>,
    pub gather_facts: GatherFacts,
    pub serial: Serial,
    pub tags: Vec<String>,
    pub pre_tasks: Vec<TaskNode>,
    pub tasks: Vec<TaskNode>,
    pub post_tasks: Vec<TaskNode>,
    pub handlers: Vec<TaskNode>,
    pub roles: Vec<RoleRef>,
    /// `vars_files:` — external YAML var files to load (relative to playbook dir).
    pub vars_files: Vec<String>,
    /// `any_errors_fatal:` — any task failure stops the entire play immediately.
    pub any_errors_fatal: Option<bool>,
    pub environment: HashMap<String, String>,
}

/// A task list entry: either a leaf task or a `block:` aggregate.
///
/// The `Task` variant is boxed: a `Task` carries many fields (module args,
/// `when`, loop spec, ...) and is ~448 B, whereas `Block` is ~224 B. Boxing
/// keeps the enum pointer-sized so `Vec<TaskNode>` stays compact.
#[derive(Debug, Clone)]
pub enum TaskNode {
    Task(Box<Task>),
    Block(Box<Block>),
}

/// `block:` / `rescue:` / `always:` aggregate.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub name: Option<String>,
    pub vars: Vars,
    /// `when:` list (AND of expressions); empty ⇒ none.
    pub when: Vec<Expr>,
    pub tasks: Vec<TaskNode>,
    pub rescue: Vec<TaskNode>,
    pub always: Vec<TaskNode>,
    pub r#become: Option<bool>,
    pub tags: Vec<String>,
}

/// A leaf task: one module invocation.
#[derive(Debug, Clone, Default)]
pub struct Task {
    pub name: Option<String>,
    pub module: ModuleRef,
    /// Free-form args (a scalar for `command: echo hi`, or a mapping for
    /// `apt: { name: foo, state: present }`). Untemplated at parse time.
    pub args: serde_yaml::Value,
    pub when: Vec<Expr>,
    pub loop_: Option<LoopSpec>,
    /// `loop_control:` directive (display options for loops).
    pub loop_control: Option<LoopControl>,
    pub vars: Vars,
    pub tags: Vec<String>,
    pub r#become: Option<bool>,
    pub become_user: Option<String>,
    pub register: Option<String>,
    pub changed_when: Vec<Expr>,
    pub failed_when: Vec<Expr>,
    pub ignore_errors: Option<bool>,
    pub no_log: Option<bool>,
    /// `delegate_to:` — run this task on a different host. When set, the executor
    /// connects to the named host instead of the play's target host.
    /// `local_action:` sets this to `"localhost"`.
    pub delegate_to: Option<String>,
    /// `run_once:` — run this task on only the first host in the batch.
    pub run_once: Option<bool>,
    pub environment: HashMap<String, String>,
    /// Handler names to notify on `changed` (Ansible runs notified handlers at
    /// the end of the play or on `meta: flush_handlers`).
    pub notify: Vec<String>,
    /// `listen:` topics on a handler — notified when a task's `notify:` includes
    /// the topic name (Ansible's `listen:` handler-dispatch mechanism).
    pub listen: Vec<String>,
}

/// Loop specification. `loop`/`with_items`, `with_dict`, `with_indexed_items`.
#[derive(Debug, Clone)]
pub enum LoopSpec {
    /// `loop:` / `with_items:` — iterate a list.
    Items(LoopSource),
    /// `with_dict:` — iterate `(key, value)` pairs.
    Dict(LoopSource),
    /// `with_indexed_items:` — `(index, item)`.
    Indexed(LoopSource),
}

/// A loop's data source: either a Jinja expression or an inline YAML literal.
#[derive(Debug, Clone)]
pub enum LoopSource {
    /// A Jinja expression string (`{{ fruits }}` or a bare name like `fruits`).
    Expr(Expr),
    /// An inline YAML literal (a list or mapping), used as-is without rendering.
    Literal(serde_yaml::Value),
}

/// `loop_control:` directive on a task (Ansible loop display/behavior options).
/// Only `label` is parsed in v0.1; other sub-keys (`index_var`, `pause`, `extended`, ...)
/// are acknowledged but not stored.
#[derive(Debug, Clone, Default)]
pub struct LoopControl {
    /// `loop_control.label:` — a Jinja template rendered per-iteration for
    /// display. When unset, Ansible shows the raw `item` value.
    pub label: Option<String>,
}

/// A `roles:` entry: a plain name or `{ role: x, vars: {...}, tags: [...] }`.
#[derive(Debug, Clone, Default)]
pub struct RoleRef {
    pub role: String,
    pub vars: Vars,
    pub tags: Vec<String>,
    pub when: Vec<Expr>,
}

/// Hosts matcher string (e.g. `"webservers"`, `"all"`, `"a,b,!c"`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostMatcher(pub String);

impl HostMatcher {
    /// The raw matcher string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Canonical module name (e.g. `"cmd"`, `"apt"`). The parser canonicalizes
/// `ansible.builtin.X` → `X` on the way in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleRef(pub String);

impl ModuleRef {
    /// The canonical module name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Jinja expression string (`when`, `changed_when`, …). Untemplated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expr(pub String);

impl Expr {
    /// The raw expression text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Ordered variable map (play/block/task `vars:`). Preserves YAML key order.
#[derive(Debug, Clone, Default)]
pub struct Vars(pub IndexMap<String, serde_yaml::Value>);

/// `gather_facts:` setting. Lenient — unknown string values are retained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GatherFacts {
    /// Ansible's implicit default.
    #[default]
    Smart,
    /// Explicit `gather_facts: true` / `false`.
    Bool(bool),
    /// `gather_facts: legacy`.
    Legacy,
    /// `gather_facts: no` / `false`-as-no.
    No,
    /// Any other string form (kept verbatim for later phases).
    Explicit(String),
}

/// `serial:` batch sizing. Kept raw for Phase 1; refined when orchestration
/// lands.
#[derive(Debug, Clone, Default)]
pub struct Serial(pub Vec<serde_yaml::Value>);

impl Play {
    /// Iterate every task node in declaration order: `pre_tasks`, roles
    /// (flattened as opaque), tasks, `post_tasks`. Used by `--list-tasks`.
    fn walk(&self) -> impl Iterator<Item = &TaskNode> {
        self.pre_tasks
            .iter()
            .chain(self.tasks.iter())
            .chain(self.post_tasks.iter())
    }
}

/// Recursively collect every leaf [`Task`] under a node (descending blocks).
#[must_use]
pub fn leaf_tasks(node: &TaskNode) -> Vec<&Task> {
    let mut out = Vec::new();
    match node {
        TaskNode::Task(t) => out.push(t.as_ref()),
        TaskNode::Block(b) => {
            for child in b.tasks.iter().chain(b.rescue.iter()).chain(b.always.iter()) {
                out.extend(leaf_tasks(child));
            }
        }
    }
    out
}

/// Every leaf task in a play, in declaration order (`pre_tasks` → tasks →
/// `post_tasks`; handlers excluded — they run on notify).
#[must_use]
pub fn play_leaf_tasks(play: &Play) -> Vec<&Task> {
    play.walk().flat_map(leaf_tasks).collect()
}
