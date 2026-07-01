//! Shared benchmark helpers.
//!
//! Provides canned playbooks (a small constant + a generated ~50-task medium
//! playbook) plus thin wrappers around [`parser::parse_playbook`] and
//! [`runner::execute`] so the bench targets only measure the parse / execute
//! paths without repeating the argument plumbing. The execute wrappers run
//! against [`test_support::null_core`] (a mock `CoreApi` that returns success
//! for every call), so no real host is contacted.

use std::fmt::Write as _;

use komandan_playbook::inventory::Inventory;
use komandan_playbook::parser::{self, Playbook, Vars as PVars};
use komandan_playbook::runner::{execute, tags::TagFilter};
use komandan_playbook::test_support::null_core;

/// A small playbook: a single play, no roles, 6 leaf tasks.
pub const SMALL_PLAYBOOK: &str = "\
---
- hosts: localhost
  gather_facts: false
  tasks:
    - name: t1
      debug: msg=hello
    - name: t2
      debug: msg=world
    - name: t3
      set_fact: x=42
    - name: t4
      debug: var=x
    - name: t5
      ping:
    - name: t6
      debug: msg=done
";

/// Build a medium playbook (~50 tasks) exercising loops, conditionals,
/// handlers, mutating modules, and tags — the realistic execution mix.
///
/// Layout (10 groups of 5 tasks):
/// - `debug` — pure templating.
/// - `set_fact` — var-layer mutation.
/// - `ping` — connection probe (mocked).
/// - `loop` — 10-item iteration over a play var.
/// - `when` — conditional gating.
/// - `file` / `copy` / `lineinfile` — reuse executors (mocked success).
/// - `assert` — control-flow truthiness check.
/// - tagged tasks that `notify` a handler (flushed at play end).
#[must_use]
pub fn medium_playbook() -> String {
    let mut out = String::new();
    out.push_str("---\n- hosts: localhost\n  gather_facts: false\n");
    out.push_str("  vars:\n    items: [a, b, c, d, e, f, g, h, i, j]\n    enable: true\n");
    out.push_str("  tasks:\n");

    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: debug-{i}\n      debug: msg=\"debug task {i}\"\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: set_fact-{i}\n      set_fact: fact_{i}=\"value{i}\"\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(out, "    - name: ping-{i}\n      ping:\n");
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: loop-{i}\n      debug: msg=\"{{{{ item }}}}\"\n      loop: \"{{{{ items }}}}\"\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: when-{i}\n      debug: msg=\"conditional {i}\"\n      when: enable\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: file-{i}\n      file: path=/tmp/komandan_bench_{i} state=touch\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: copy-{i}\n      copy: dest=/tmp/komandan_bench_copy_{i} content=\"hello {i}\"\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: lineinfile-{i}\n      lineinfile: path=/tmp/komandan_bench_li_{i} line=\"line {i}\" create=yes\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: assert-{i}\n      assert:\n        that:\n          - enable\n"
        );
    }
    for i in 1..=5 {
        let _ = write!(
            out,
            "    - name: tagged-{i}\n      debug: msg=\"tagged {i}\"\n      tags: [bench]\n      notify: restart handler\n"
        );
    }

    out.push_str("  handlers:\n    - name: restart handler\n      debug: msg=\"handler ran\"\n");
    out
}

/// Parse a playbook YAML string into the single-element slice [`execute`]
/// expects. Returns an empty vec on parse failure so a malformed fixture does
/// not panic the harness.
#[must_use]
pub fn parse(pb_yaml: &str) -> Vec<(String, Playbook)> {
    parser::parse_playbook(pb_yaml)
        .map_or_else(|_| Vec::new(), |pb| vec![("bench.yml".to_string(), pb)])
}

/// Run a parsed playbook slice against the mock core with `forks = 1`.
#[allow(dead_code)] // used by the `playbook` bench; unused when only `parse` bench runs.
#[must_use]
pub fn run_parsed(plays: &[(String, Playbook)]) -> String {
    run_parsed_with_forks(plays, 1)
}

/// Run a parsed playbook slice against the mock core with a custom `forks`
/// value (sizing the dedicated rayon thread pool).
#[allow(dead_code)] // used by the `playbook` bench; unused when only `parse` bench runs.
#[must_use]
pub fn run_parsed_with_forks(plays: &[(String, Playbook)], forks: usize) -> String {
    execute(
        plays,
        &Inventory::implicit_localhost(),
        None,
        &null_core(),
        false,
        &TagFilter::none(),
        None,
        &PVars::default(),
        forks,
        false,
        false,
    )
    .unwrap_or_default()
}
