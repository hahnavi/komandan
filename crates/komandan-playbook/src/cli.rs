//! The playbook plugin's own CLI (re-parsed from the host's argv tail).
//!
//! Mirrors `docs/PLAYBOOK_SPEC.md` §3. Phase 1 declares only the flags the
//! listing/syntax commands consume; execution flags (`--forks`, `--become`,
//! ...) land with the phases that implement them. The `-e`/`--extra-vars`
//! flag is accepted now (collected for the templating layer); execution
//! semantics arrive with the runner phase.
//!
//! The host hands the plugin the raw argv tail *after* the subcommand name; we
//! prepend a synthetic bin name so [`clap::Parser::try_parse_from`] sees a
//! conventional argv (argv[0] = program name).

use clap::Parser;

/// `komandan playbook` — run Ansible-format playbooks via komandan's Rust core.
#[derive(Parser, Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // CLI flag switches; bools are idiomatic here.
#[command(
    name = "komandan playbook",
    about = "Parse/list/run Ansible-format playbooks via komandan's Rust core",
    version
)]
pub struct PlaybookArgs {
    /// One or more playbook files, run in order.
    #[arg(required = true, num_args = 1..)]
    pub playbooks: Vec<String>,

    /// Inventory file or comma-separated host list. Default: implicit localhost.
    #[arg(short = 'i', long = "inventory")]
    pub inventory: Option<String>,

    /// Limit hosts (a host pattern: group, host, comma list).
    #[arg(short = 'l', long = "limit")]
    pub limit: Option<String>,

    /// Parse + validate everything; print nothing on success.
    #[arg(long = "syntax-check")]
    pub syntax_check: bool,

    /// Print resolved hosts per play; do nothing else.
    #[arg(long = "list-hosts")]
    pub list_hosts: bool,

    /// Print the task list per play; do nothing else.
    #[arg(long = "list-tasks")]
    pub list_tasks: bool,

    /// Dry run: executors honoring check-mode report changes without applying.
    #[arg(long = "check")]
    pub check: bool,

    /// Only run tasks tagged with these (comma-separated).
    #[arg(short = 't', long = "tags")]
    pub tags: Option<String>,

    /// Skip tasks tagged with these (comma-separated).
    #[arg(long = "skip-tags")]
    pub skip_tags: Option<String>,

    /// Start execution at the task matching this name.
    #[arg(long = "start-at-task")]
    pub start_at_task: Option<String>,

    /// Set variables (e.g. `-e key=value`, `-e @vars.yml`, or `-e '{"k":"v"}'`).
    /// May be specified multiple times; later values override earlier ones.
    #[arg(short = 'e', long = "extra-vars")]
    pub extra_vars: Vec<String>,

    /// Number of parallel hosts per batch (default 5, like Ansible).
    #[arg(short = 'f', long = "forks", default_value = "5")]
    pub forks: usize,

    /// Show unified diffs for file changes (Ansible `--diff`).
    #[arg(long = "diff")]
    pub diff: bool,

    /// Skip tasks using unsupported modules with a warning instead of failing.
    #[arg(
        long = "skip-unsupported",
        long_help = "Skip tasks that reference modules\nnot implemented in this komandan build, emitting a warning\ninstead of failing the play."
    )]
    pub skip_unsupported: bool,
}
