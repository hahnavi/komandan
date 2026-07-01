//! YAML → IR parser.
//!
//! Walks the [`serde_yaml::Value`] tree explicitly rather than deriving
//! `Deserialize`, because Ansible's task model does not map cleanly onto
//! derive: a task mapping's unknown top-level key *is* the module name and its
//! value the args (see [`parse_task`]). Derived deserialization cannot express
//! "the remaining key is the module".
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §4. Strict on structural errors (a play
//! without `hosts:`, a `block:` without its inner task list, a task with no
//! module key); lenient on unknown module args (passed through as raw YAML).
//!
//! # Errors
//!
//! Every fallible entry point returns [`ParseError`] (see [`error`]).
//! [`parse_playbook`] is the top-level entry used by the listing commands.

use serde_yaml::{Mapping, Value};

use super::model::{
    Block, Expr, GatherFacts, HostMatcher, LoopControl, LoopSource, LoopSpec, ModuleRef, Play,
    Playbook, RoleRef, Serial, Task, TaskNode, Vars,
};
use crate::error::ParseError;

/// Parse a standalone task-list file (a YAML sequence of task/block mappings,
/// as used by `include_tasks`, `import_tasks`, and role `tasks/main.yml`).
///
/// # Errors
///
/// [`ParseError::Yaml`] on malformed YAML; [`ParseError::Task`] on a bad task.
pub fn parse_tasks_text(text: &str) -> Result<Vec<TaskNode>, ParseError> {
    let doc: Value = serde_yaml::from_str(text).map_err(ParseError::yaml)?;
    match unwrap_tagged(&doc) {
        Value::Null => Ok(Vec::new()),
        Value::Sequence(_) => parse_task_list(Some(&doc)),
        other => Err(ParseError::task(format!(
            "task list must be a sequence, got {}",
            yaml_kind(other)
        ))),
    }
}

/// Parse a standalone vars file (a YAML flat mapping of `key: value`, as used
/// by role `vars/main.yml`, `defaults/main.yml`, `group_vars/`, `host_vars/`,
/// and `vars_files:`).
///
/// # Errors
///
/// [`ParseError::Yaml`] on malformed YAML; [`ParseError::Play`] if not a
/// mapping.
pub fn parse_vars_text(text: &str) -> Result<Vars, ParseError> {
    let doc: Value = serde_yaml::from_str(text).map_err(ParseError::yaml)?;
    match unwrap_tagged(&doc) {
        Value::Mapping(_) => Ok(parse_vars(&doc)),
        Value::Null => Ok(Vars::default()),
        other => Err(ParseError::play(format!(
            "vars file must be a mapping, got {}",
            yaml_kind(other)
        ))),
    }
}

/// Parse a playbook document (a YAML sequence of play mappings, or a single
/// play mapping).
///
/// # Errors
///
/// Returns [`ParseError::Yaml`] on malformed YAML, or [`ParseError::Play`] if
/// a play is structurally invalid (missing `hosts:`, a bad task, ...).
pub fn parse_playbook(text: &str) -> Result<Playbook, ParseError> {
    let doc: Value = serde_yaml::from_str(text).map_err(ParseError::yaml)?;
    let plays = match unwrap_tagged(&doc) {
        Value::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(parse_play(item)?);
            }
            out
        }
        Value::Mapping(_) => vec![parse_play(&doc)?],
        Value::Null => Vec::new(),
        other => {
            return Err(ParseError::play(format!(
                "playbook must be a sequence of plays, got {}",
                yaml_kind(other)
            )));
        }
    };
    Ok(Playbook(plays))
}

/// Parse a single play mapping.
fn parse_play(value: &Value) -> Result<Play, ParseError> {
    let mapping = require_mapping(value).map_err(ParseError::play)?;
    // `import_playbook` is a special top-level directive, not a play — no
    // `hosts:` required. The referenced file is loaded and its plays spliced
    // in-place by `parse_playbook_file`.
    if let Some(v) = map_get(&mapping, "import_playbook") {
        let path = parse_string(v)
            .ok_or_else(|| ParseError::play("import_playbook must be a string path".to_string()))?;
        return Ok(Play {
            import_playbook: Some(path),
            ..Default::default()
        });
    }
    let hosts = map_get(&mapping, "hosts")
        .map(parse_host_matcher)
        .transpose()?;
    if hosts.is_none() {
        return Err(ParseError::play(
            "play is missing required `hosts:` key".to_string(),
        ));
    }

    let name = map_get(&mapping, "name")
        .filter(|v| !matches!(unwrap_tagged(v), Value::Null))
        .and_then(parse_string);
    let vars = map_get(&mapping, "vars")
        .map(parse_vars)
        .unwrap_or_default();
    let r#become = map_get(&mapping, "become").and_then(parse_bool);
    let become_user = map_get(&mapping, "become_user").and_then(parse_string);
    let remote_user = map_get(&mapping, "remote_user").and_then(parse_string);
    let gather_facts = map_get(&mapping, "gather_facts")
        .map(parse_gather_facts)
        .unwrap_or_default();
    let serial = map_get(&mapping, "serial")
        .map(parse_serial)
        .unwrap_or_default();
    let tags = map_get(&mapping, "tags")
        .map(parse_string_list)
        .unwrap_or_default();
    let environment = map_get(&mapping, "environment")
        .map(parse_env)
        .unwrap_or_default();
    let pre_tasks = parse_task_list(map_get(&mapping, "pre_tasks"))?;
    let tasks = parse_task_list(map_get(&mapping, "tasks"))?;
    let post_tasks = parse_task_list(map_get(&mapping, "post_tasks"))?;
    let handlers = parse_task_list(map_get(&mapping, "handlers"))?;
    let roles = map_get(&mapping, "roles")
        .map(parse_roles)
        .unwrap_or_default();
    let vars_files = map_get(&mapping, "vars_files")
        .map(parse_string_list)
        .unwrap_or_default();
    let any_errors_fatal = map_get(&mapping, "any_errors_fatal").and_then(parse_bool);

    Ok(Play {
        import_playbook: None,
        hosts,
        name,
        vars,
        r#become,
        become_user,
        remote_user,
        gather_facts,
        serial,
        tags,
        pre_tasks,
        tasks,
        post_tasks,
        handlers,
        roles,
        vars_files,
        any_errors_fatal,
        environment,
    })
}

/// Parse a `Vec<TaskNode>` from an optional YAML value.
fn parse_task_list(value: Option<&Value>) -> Result<Vec<TaskNode>, ParseError> {
    let Some(v) = value else {
        return Ok(Vec::new());
    };
    match unwrap_tagged(v) {
        Value::Null => Ok(Vec::new()),
        Value::Sequence(items) => items.iter().map(parse_task_node).collect(),
        other => Err(ParseError::task(format!(
            "task list must be a sequence, got {}",
            yaml_kind(other)
        ))),
    }
}

/// Parse one task-list node: a `block:` aggregate or a leaf task.
fn parse_task_node(value: &Value) -> Result<TaskNode, ParseError> {
    let mapping = require_mapping(value).map_err(ParseError::task)?;
    if mapping.get(Value::String("block".to_string())).is_some() {
        parse_block(&mapping).map(|b| TaskNode::Block(Box::new(b)))
    } else {
        parse_task(&mapping).map(|t| TaskNode::Task(Box::new(t)))
    }
}

/// Parse a `block:` aggregate.
fn parse_block(mapping: &Mapping) -> Result<Block, ParseError> {
    let name = map_get(mapping, "name").and_then(parse_string);
    let vars = map_get(mapping, "vars").map(parse_vars).unwrap_or_default();
    let when = map_get(mapping, "when").map(parse_when).unwrap_or_default();
    let tasks = parse_task_list(map_get(mapping, "block"))?;
    let rescue = parse_task_list(map_get(mapping, "rescue"))?;
    let always = parse_task_list(map_get(mapping, "always"))?;
    let r#become = map_get(mapping, "become").and_then(parse_bool);
    let tags = map_get(mapping, "tags")
        .map(parse_string_list)
        .unwrap_or_default();
    Ok(Block {
        name,
        vars,
        when,
        tasks,
        rescue,
        always,
        r#become,
        tags,
    })
}

/// Parse a leaf task. The module name is the single non-reserved key (Ansible's
/// rule); `action:` shorthand is also honoured.
fn parse_task(mapping: &Mapping) -> Result<Task, ParseError> {
    let name = map_get(mapping, "name").and_then(parse_string);
    let when = map_get(mapping, "when").map(parse_when).unwrap_or_default();
    let loop_ = parse_loop(mapping);
    let loop_control = map_get(mapping, "loop_control").map(parse_loop_control);
    let vars = map_get(mapping, "vars").map(parse_vars).unwrap_or_default();
    let tags = map_get(mapping, "tags")
        .map(parse_string_list)
        .unwrap_or_default();
    let r#become = map_get(mapping, "become").and_then(parse_bool);
    let become_user = map_get(mapping, "become_user").and_then(parse_string);
    let register = map_get(mapping, "register").and_then(parse_string);
    let changed_when = map_get(mapping, "changed_when")
        .map(parse_when)
        .unwrap_or_default();
    let failed_when = map_get(mapping, "failed_when")
        .map(parse_when)
        .unwrap_or_default();
    let ignore_errors = map_get(mapping, "ignore_errors").and_then(parse_bool);
    let no_log = map_get(mapping, "no_log").and_then(parse_bool);
    let environment = map_get(mapping, "environment")
        .map(parse_env)
        .unwrap_or_default();
    let notify = map_get(mapping, "notify")
        .map(parse_string_list)
        .unwrap_or_default();
    let listen = map_get(mapping, "listen")
        .map(parse_string_list)
        .unwrap_or_default();

    let delegate_to = map_get(mapping, "delegate_to").and_then(parse_string);
    let run_once = map_get(mapping, "run_once").and_then(parse_bool);
    // `local_action` is module shorthand that also implies `delegate_to: localhost`.
    let is_local_action = map_get(mapping, "local_action").is_some();

    let (module, args) = resolve_module(mapping)?;

    Ok(Task {
        name,
        module,
        args,
        when,
        loop_,
        loop_control,
        vars,
        tags,
        r#become,
        become_user,
        register,
        changed_when,
        failed_when,
        ignore_errors,
        no_log,
        delegate_to: if is_local_action {
            delegate_to.or_else(|| Some("localhost".to_string()))
        } else {
            delegate_to
        },
        run_once,
        environment,
        notify,
        listen,
    })
}

/// Resolve a task's module + args.
///
/// Order: `action:` shorthand → `local_action:` → the single non-reserved key
/// (Ansible's rule). Unknown reserved keys are ignored; multiple module keys
/// is a structural error.
fn resolve_module(mapping: &Mapping) -> Result<(ModuleRef, Value), ParseError> {
    if let Some(action) = map_get(mapping, "action") {
        return parse_action(action);
    }
    if let Some(action) = map_get(mapping, "local_action") {
        return parse_action(action);
    }
    let mut module_keys: Vec<String> = Vec::new();
    for (k, v) in mapping {
        // A null value is a legitimate no-arg module (e.g. `ping:`,
        // `setup:`); reserved keys are skipped by `is_reserved_task_key`
        // below, so do not reject null-valued keys here (they would otherwise
        // be wrongly dropped, breaking no-arg modules).
        let _ = v;
        if let Value::String(s) = unwrap_tagged(k) {
            if is_reserved_task_key(s) {
                continue;
            }
            module_keys.push(s.clone());
        }
    }
    match module_keys.len() {
        0 => Err(ParseError::task(
            "task has no module key (expected one action such as `cmd:`, `apt:`, ...)".to_string(),
        )),
        1 => {
            let key = module_keys[0].clone();
            let val = mapping
                .get(Value::String(key.clone()))
                .cloned()
                .unwrap_or(Value::Null);
            Ok((ModuleRef(canonicalize_module(&key)), val))
        }
        _ => Err(ParseError::task(format!(
            "task has multiple module keys ({module_keys:?}); only one action per task is allowed"
        ))),
    }
}

/// Parse `action:` shorthand. Forms: `action: cmd` (bare name) or
/// `action: { module: cmd, args: {...} }`.
fn parse_action(value: &Value) -> Result<(ModuleRef, Value), ParseError> {
    match unwrap_tagged(value) {
        Value::String(s) => Ok((ModuleRef(canonicalize_module(s)), Value::Null)),
        Value::Mapping(m) => {
            let module = map_get(m, "module").and_then(parse_string).ok_or_else(|| {
                ParseError::task("action: shorthand missing `module:`".to_string())
            })?;
            let args = map_get(m, "args").cloned().unwrap_or(Value::Null);
            Ok((ModuleRef(canonicalize_module(&module)), args))
        }
        other => Err(ParseError::task(format!(
            "action: must be a string or mapping, got {}",
            yaml_kind(other)
        ))),
    }
}

/// Detect a loop spec from `loop`/`with_items`/`with_dict`/`with_indexed_items`.
fn parse_loop(mapping: &Mapping) -> Option<LoopSpec> {
    if let Some(v) = map_get(mapping, "loop").or_else(|| map_get(mapping, "with_items")) {
        return Some(LoopSpec::Items(parse_source(v)));
    }
    if let Some(v) = map_get(mapping, "with_dict") {
        return Some(LoopSpec::Dict(parse_source(v)));
    }
    if let Some(v) = map_get(mapping, "with_indexed_items") {
        return Some(LoopSpec::Indexed(parse_source(v)));
    }
    None
}

/// Build a [`LoopSource`]: a scalar becomes a Jinja [`LoopSource::Expr`]; a
/// literal list/mapping becomes [`LoopSource::Literal`].
fn parse_source(v: &Value) -> LoopSource {
    parse_string(v).map_or_else(
        || LoopSource::Literal(v.clone()),
        |s| LoopSource::Expr(Expr(s)),
    )
}

/// Parse a `loop_control:` mapping into a [`LoopControl`].
fn parse_loop_control(value: &Value) -> LoopControl {
    let label = value.as_mapping().and_then(|m| {
        m.get(Value::String("label".to_string()))
            .and_then(parse_string)
    });
    LoopControl { label }
}

/// Parse `roles:` into [`RoleRef`]s. Accepts plain names or mapping form.
fn parse_roles(value: &Value) -> Vec<RoleRef> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items.iter().filter_map(parse_one_role).collect(),
        Value::String(s) => vec![RoleRef {
            role: s.clone(),
            vars: Vars::default(),
            tags: Vec::new(),
            when: Vec::new(),
        }],
        _ => Vec::new(),
    }
}

fn parse_one_role(value: &Value) -> Option<RoleRef> {
    match unwrap_tagged(value) {
        Value::String(s) => Some(RoleRef {
            role: s.clone(),
            vars: Vars::default(),
            tags: Vec::new(),
            when: Vec::new(),
        }),
        Value::Mapping(m) => {
            let role = map_get(m, "role").and_then(parse_string)?;
            let vars = map_get(m, "vars").map(parse_vars).unwrap_or_default();
            let tags = map_get(m, "tags")
                .map(parse_string_list)
                .unwrap_or_default();
            let when = map_get(m, "when").map(parse_when).unwrap_or_default();
            Some(RoleRef {
                role,
                vars,
                tags,
                when,
            })
        }
        _ => None,
    }
}

// ---- scalar / collection helpers ----------------------------------------

fn parse_host_matcher(value: &Value) -> Result<HostMatcher, ParseError> {
    parse_string(value).map(HostMatcher).ok_or_else(|| {
        ParseError::play(format!(
            "`hosts:` must be a string, got {}",
            yaml_kind(unwrap_tagged(value))
        ))
    })
}

fn parse_vars(value: &Value) -> Vars {
    match unwrap_tagged(value) {
        Value::Mapping(m) => {
            let mut out = indexmap::IndexMap::new();
            for (k, v) in m {
                if let Value::String(key) = unwrap_tagged(k) {
                    out.insert(key.clone(), v.clone());
                }
            }
            Vars(out)
        }
        _ => Vars::default(),
    }
}

fn parse_env(value: &Value) -> std::collections::HashMap<String, String> {
    match unwrap_tagged(value) {
        Value::Mapping(m) => {
            let mut out = std::collections::HashMap::new();
            for (k, v) in m {
                if let (Value::String(key), Some(val)) = (unwrap_tagged(k), parse_string(v)) {
                    out.insert(key.clone(), val);
                }
            }
            out
        }
        _ => std::collections::HashMap::new(),
    }
}

/// `when:` accepts a single string or a list of strings (AND). Non-string
/// (e.g. boolean) values coerce to their YAML rendering.
fn parse_when(value: &Value) -> Vec<Expr> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items
            .iter()
            .filter_map(|v| parse_string(v).map(Expr))
            .collect(),
        Value::Null => Vec::new(),
        other => parse_string(other)
            .map(|s| vec![Expr(s)])
            .unwrap_or_default(),
    }
}

fn parse_string_list(value: &Value) -> Vec<String> {
    match unwrap_tagged(value) {
        Value::Sequence(items) => items.iter().filter_map(parse_string).collect(),
        Value::String(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_gather_facts(value: &Value) -> GatherFacts {
    match unwrap_tagged(value) {
        Value::Bool(true) => GatherFacts::Bool(true),
        Value::Bool(false) => GatherFacts::No,
        Value::String(s) => match s.as_str() {
            "smart" => GatherFacts::Smart,
            "legacy" => GatherFacts::Legacy,
            "no" | "No" | "NO" => GatherFacts::No,
            other => GatherFacts::Explicit(other.to_string()),
        },
        Value::Null => GatherFacts::Smart,
        other => GatherFacts::Explicit(yaml_kind(other).to_string()),
    }
}

fn parse_serial(value: &Value) -> Serial {
    match unwrap_tagged(value) {
        Value::Sequence(items) => Serial(items.clone()),
        Value::Null => Serial::default(),
        other => Serial(vec![other.clone()]),
    }
}

fn parse_string(value: &Value) -> Option<String> {
    match unwrap_tagged(value) {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn parse_bool(value: &Value) -> Option<bool> {
    match unwrap_tagged(value) {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.as_str() {
            "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" => Some(true),
            "no" | "No" | "NO" | "false" | "False" | "FALSE" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

// ---- YAML plumbing ------------------------------------------------------

/// Strip a `Value::Tagged` wrapper, if any (YAML tags like `!!str`).
fn unwrap_tagged(value: &Value) -> &Value {
    match value {
        Value::Tagged(inner) => unwrap_tagged(&inner.value),
        _ => value,
    }
}

fn v_is_null(value: &Value) -> bool {
    matches!(unwrap_tagged(value), Value::Null)
}

fn require_mapping(value: &Value) -> Result<Mapping, String> {
    match unwrap_tagged(value) {
        Value::Mapping(m) => Ok(m.clone()),
        other => Err(format!("expected a mapping, got {}", yaml_kind(other))),
    }
}

/// String-keyed lookup on a YAML mapping (returns `None` for absent or null).
fn map_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    let v = mapping.get(Value::String(key.to_string()))?;
    if v_is_null(v) { None } else { Some(v) }
}

const fn yaml_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "sequence",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged",
    }
}

/// Canonicalize a module name: `ansible.builtin.X` / `ansible.legacy.X` → `X`.
/// Collection-qualified names (`community.docker.X`) are left intact.
fn canonicalize_module(name: &str) -> String {
    for prefix in ["ansible.builtin.", "ansible.legacy."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    name.to_string()
}

/// Is `key` a reserved (non-module) task attribute key?
fn is_reserved_task_key(key: &str) -> bool {
    matches!(
        key,
        "name"
            | "when"
            | "loop"
            | "with_items"
            | "with_dict"
            | "with_indexed_items"
            | "with_filetree"
            | "with_fileglob"
            | "loop_control"
            | "vars"
            | "tags"
            | "become"
            | "become_user"
            | "become_method"
            | "register"
            | "changed_when"
            | "failed_when"
            | "ignore_errors"
            | "no_log"
            | "environment"
            | "action"
            | "local_action"
            | "delegate_to"
            | "delegate_facts"
            | "run_once"
            | "connection"
            | "retries"
            | "until"
            | "delay"
            | "notify"
            | "listen"
            | "module_defaults"
            | "check_mode"
            | "diff"
            | "block"
            | "rescue"
            | "always"
            | "throttle"
            | "poll"
            | "async"
            | "ansible_async"
            | "ansible_check_mode"
            | "any_errors_fatal"
            | "max_fail_percentage"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_task_delegate_to() -> anyhow::Result<()> {
        let mapping: Mapping = serde_yaml::from_str("command: echo hi\ndelegate_to: localhost\n")?;
        let task = parse_task(&mapping)?;
        assert_eq!(task.delegate_to.as_deref(), Some("localhost"));
        Ok(())
    }

    #[test]
    fn parse_task_run_once() -> anyhow::Result<()> {
        let mapping: Mapping = serde_yaml::from_str("command: echo hi\nrun_once: true\n")?;
        let task = parse_task(&mapping)?;
        assert_eq!(task.run_once, Some(true));
        Ok(())
    }

    #[test]
    fn local_action_sets_delegate_to() -> anyhow::Result<()> {
        let mapping: Mapping =
            serde_yaml::from_str("local_action:\n  module: command\n  args:\n    cmd: echo hi\n")?;
        let task = parse_task(&mapping)?;
        assert_eq!(task.delegate_to.as_deref(), Some("localhost"));
        assert_eq!(task.module.as_str(), "command");
        Ok(())
    }

    #[test]
    fn parse_play_vars_files() -> anyhow::Result<()> {
        let pb = parse_playbook(
            "- hosts: all\n  vars_files:\n    - vars/common.yml\n    - vars/prod.yml\n",
        )?;
        assert_eq!(pb.0[0].vars_files, vec!["vars/common.yml", "vars/prod.yml"]);
        Ok(())
    }

    #[test]
    fn parse_play_any_errors_fatal() -> anyhow::Result<()> {
        let pb = parse_playbook("- hosts: all\n  any_errors_fatal: true\n")?;
        assert_eq!(pb.0[0].any_errors_fatal, Some(true));
        Ok(())
    }

    #[test]
    fn parse_task_loop_control_label() -> anyhow::Result<()> {
        let mapping: Mapping = serde_yaml::from_str(
            "debug: msg=\"{{ item }}\"\nloop:\n  - 1\nloop_control:\n  label: \"{{ item }}\"\n",
        )?;
        let task = parse_task(&mapping)?;
        assert!(task.loop_control.is_some());
        assert_eq!(
            task.loop_control
                .as_ref()
                .and_then(|lc| lc.label.as_deref()),
            Some("{{ item }}")
        );
        Ok(())
    }

    #[test]
    fn parse_task_loop_control_absent() -> anyhow::Result<()> {
        let mapping: Mapping = serde_yaml::from_str("debug: msg=hi\n")?;
        let task = parse_task(&mapping)?;
        assert!(task.loop_control.is_none());
        Ok(())
    }

    #[test]
    fn parse_task_loop_control_without_label() -> anyhow::Result<()> {
        let mapping: Mapping =
            serde_yaml::from_str("debug: msg=hi\nloop:\n  - 1\nloop_control:\n  pause: 5\n")?;
        let task = parse_task(&mapping)?;
        assert!(task.loop_control.is_some());
        assert!(
            task.loop_control
                .as_ref()
                .and_then(|lc| lc.label.as_deref())
                .is_none()
        );
        Ok(())
    }
}
