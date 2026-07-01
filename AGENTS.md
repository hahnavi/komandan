# AGENTS.md

Guide for AI agents (and humans) working on Komandan. Read before editing.

## What is Komandan

Agentless server-automation tool. Embeds LuaJIT (via `mlua`) as the scripting
layer; Rust provides the runtime — connection factory, 14 built-in modules,
parallel executor, system checks. Users write `*.lua` task scripts; Rust
executes them over SSH or locally. Think "Ansible, but Lua-scripted and
written in Rust". Current version `0.0.4`.

Status: a **Cargo workspace** (see Architecture). Phase 0 of the plugin
system is landed — the host loads cdylib
plugins via `abi_stable` + `libloading`, with `komandan-hello-plugin` as the
reference plugin (try `komandan hello`). `CoreApi` is fully wired (bar
`worker_lua`/`now_playing_task` v0.1 placeholders), including `komando` via a
per-thread plugin Lua VM. **Playbook plugin Phase 1 + 2 + 3 + 4 + 5 (partial)**
is landed: `komandan-playbook` parses Ansible-format playbooks
(`--syntax-check` / `--list-hosts` / `--list-tasks` via `komandan playbook
<file...>`), ships a full templating layer (`minijinja` engine, layered `Vars`
store, the §7.3 gap filters/tests/lookups, magic vars + `omit`, `set_fact`/
`register` plumbing), **and** runs them natively — a `ModuleExecutor` registry
+ `Connection`/`ConnectionPool` over the host `CoreApi`, 14 reuse shims
(Ansible→komandan arg translation via `komando()`), the control-flow
executors (`debug`/`ping`/`set_fact`/`assert`/`fail`/`meta`/`pause`), the full
§6.3 gap set (`hostname`/`timezone`/`git`/`pip`/`stat`/`known_hosts`/
`blockinfile`/`replace`/`wait_for`/`unarchive`/`archive`/`cron`/`mount`/
`reboot`/`uri`), a per-task runner (`when`→render→`k=v` expansion→run→
register→`changed_when`/`failed_when`→report), **play orchestration** —
`block:`/`rescue:`/`always:`, loops (`loop`/`with_items`/`with_dict`/
`with_indexed_items`), handlers + `notify:` (incl. `listen:` topics; flush at
play end / `meta: flush_handlers`), `serial:` batching, tag filtering
(`--tags`/`--skip-tags`), `--start-at-task`, shell-based `gather_facts`,
**and roles** — `roles/<name>/{tasks,handlers,vars,defaults,meta}/` loading
with `meta/main.yml` dependency topological sort, role defaults/vars as var
layers, role handler merge, `RoleRef` tags/when/vars via block wrapper, `include_tasks`/`import_tasks`
(static plan-time), `include_role`/`import_role` (task-level role
invocation), `import_playbook` (recursive play splicing with cycle
detection), `group_vars`/`host_vars` directory loading, and `--list-tasks`
role expansion. Post-Phase-5 hardening landed: `-e`/`--extra-vars`,
`vars_files:`, `--check` mode (mutating modules skipped), `no_log:`,
`delegate_to:`/`local_action:`, `environment:` (env var prefix on shell
commands), `become:`/`become_user:` (play-level + task-level via
`become_prefix()`/`shell_prefix()`), `run_once:`,
`any_errors_fatal:`, `--forks` parallelism (rayon), templated includes
(`include_tasks: "{{ var }}.yml"`), `loop_control.label` (parsed + displayed
— per-item status lines), `--diff` mode (unified diffs for `blockinfile`/
`replace` via `similar` crate, plus `lineinfile`/`template`/`copy` via
before/after `cat`; `--check` mode now skips writes in gap_files + copy
executors), `add_host`/`group_by` (runtime inventory via
`RuntimeInventory`/`RuntimeAdditions` shared across plays), `--check` mode for
ALL reuse executors (stub returns `changed:true` without host contact),
templating expansion (+11 filters: `hash`/`to_nice_json`/`to_nice_yaml`/
`from_yaml_all`/`path_join`/`strftime`/`to_datetime`/`urlencode`/`urlsplit`/
`type_debug`/`flatten(depth)`; +6 lookups: `password`/`subelements`/
`flattened`/`lines`/`template`/`first_found`; +11 P2 filters:
`human_readable`/`human_to_bytes`/`center`/`win_basename`/`win_dirname`/
`win_splitext`/`regex_findall_ind`/`random`/`ipv4`/`ipv6`/`ipwrap`; +7 P2
lookups: `ini`/`json`/`sequence`/`together`/`indexed_items`/`csvfile`/
`fileglob`), `--skip-unsupported` flag (tasks with unimplemented modules skip
with warning instead of failing), **benchmark harness** (Criterion: parse +
execute benches against mock core), **and scoped-role CI** (spec
§11.3 — synthetic fixtures for geerlingguy.docker/nginx +
dev-sec.ssh-hardening patterns, per-role allow-list configs, integration test
+ CI job). Phase 5 is **complete**. Real-world role CI is wired in:
`crates/komandan-playbook/tests/real_roles.rs` clones the live
geerlingguy.docker/nginx + dev-sec.ssh-hardening repos, asserts
supported-module coverage (≥50 % per role), and runs each in check-mode via
`--skip-unsupported`; `scripts/analyze-real-roles.sh` is the standalone
shell harness (clone → `--syntax-check`/`--list-tasks` → coverage table).
Both run in the `real-roles` CI job (`continue-on-error` — upstream drift).
Phase 5 + Phase 6 are **complete**; `install.sh`, `NOTICE`, `RELEASE.md`, and
the ansible-playbook parity study have
landed. The `--syntax-check` role-task count bug (D3) is fixed.

## Toolchain

- **Rust nightly**, edition 2024. Pinned via `rust-toolchain.toml`.
- `clippy.toml` sets `too-many-lines-threshold = 500`.
- Lints are denied workspace-wide via `[workspace.lints]` in the root
  `Cargo.toml`: `unsafe_code`, `clippy::pedantic`, `clippy::nursery`,
  `clippy::unwrap_used`, `clippy::expect_used`, `clippy::enum_glob_use`. Each
  member crate opts in via `[lints] workspace = true`.
- Any new code MUST compile under `cargo clippy --workspace --all-targets -- -D warnings`
    with zero findings.

## Commands

```bash
cargo build --workspace --all-targets      # compile everything (first build ~2-3 min: LuaJIT + libssh2)
cargo clippy --workspace --all-targets -- -D warnings   # lint gate — MUST be clean
cargo fmt                                  # format; --check in CI
cargo test --workspace --all-targets       # full suite across all crates
cargo test -p komandan-core --lib          # core unit tests only (fast)
cargo run -p komandan -- hello             # run the binary (default-member); dispatches `hello` plugin if loaded
cargo run -p komandan -- playbook site.yml --syntax-check   # dispatches the `playbook` plugin if loaded
```

`default-members = ["crates/komandan"]`, so bare `cargo build`/`cargo run` at
the workspace root target the binary. Use `--workspace` / `-p <crate>` to hit
the rest. Plugins are discovered from `$KOMANDAN_PLUGIN_DIR` (or
`$XDG_CONFIG_HOME/komandan/plugins`, else `~/.config/komandan/plugins`).
Trailing args after a plugin name are forwarded verbatim
(`komandan <plugin> <args...>`); the plugin owns its own arg parsing. Note:
`komandan <plugin> --help` currently shows the **host** help (clap owns the
global `-h`/`--help`); use `komandan --version` to list loaded plugins.

`tests/check_system_integration.rs::test_package_validation_with_real_packages`
is environment-dependent (needs a real package manager + specific packages).
It may fail on stripped CI runners — pre-existing, not a regression.

## Architecture

```
komandan/                         — Cargo workspace root
├── Cargo.toml                    — [workspace], [workspace.package], [workspace.lints],
│                                   [workspace.dependencies], [patch.crates-io], [profile.release]
├── crates/
│   ├── komandan-core/            — the library (rlib + cdylib); LIB NAME `komandan`
│   │   ├── build.rs              — embeds `git describe` for --version
│   │   ├── src/                  — the former top-level src/ tree (unchanged):
│   │   │   ├── lib.rs            — create_lua[_with_args]() / setup_komandan_table() / REPL
│   │   │   ├── args.rs           — clap CLI definition (Args, Flags, Commands)
│   │   │   ├── models.rs         — Host, Task, Module, KomandoResult, KomandanConfig
│   │   │   ├── executor.rs       — CommandExecutor trait (impl by SSHSession + LocalSession)
│   │   │   ├── komando.rs        — komando() + komando_parallel_{tasks,hosts}(); worker Lua
│   │   │   │                       via thread_local! OnceCell<Lua> (one VM per rayon worker)
│   │   │   ├── local.rs          — LocalSession (shells out locally)
│   │   │   ├── ssh.rs            — SSHSession (wraps libssh2 via `ssh2` crate)
│   │   │   ├── connection/       — create_connection() factory + SSH/local selection, tests
│   │   │   ├── parallel_executor/ — generic map/each/reduce framework (rayon)
│   │   │   ├── defaults.rs       — global Defaults singleton (OnceLock + RwLock)
│   │   │   ├── modules/          — one file per built-in module (see list below) + base.rs
│   │   │   │   └── core.rs       — collect_core_modules() registers all 14 modules
│   │   │   ├── checks/           — system validation: base/, file/, package/, service/
│   │   │   ├── validator.rs      — Lua-table validators for host & task
│   │   │   ├── report.rs         — execution report accumulator
│   │   │   ├── project.rs        — `project init` / `project new` scaffolding
│   │   │   ├── repl_config.rs    — REPL config from komandan/repl.conf (rustyline settings)
│   │   │   ├── plugin/           — DYNAMIC PLUGIN HOST SIDE (Phase 0)
│   │   │   │   ├── mod.rs        — PluginRegistry, load_dir, dispatch, discover_dir
│   │   │   │   ├── loader.rs     — THE ONLY `unsafe` IN THE CRATE (dlopen via libloading)
│   │   │   │   └── core_api.rs   — stub CoreApi/LoggerSink impls (real wiring = Phase 1)
│   │   │   ├── templates/        — scaffolding assets: hosts.lua, main.lua, komandan.json.j2
│   │   │   └── util/             — host_info, filter_hosts, parse_hosts_json_*, dprint, ...
│   │   ├── tests/                — integration tests (`use komandan::...`)
│   │   └── examples/             — .lua task samples + gen_module_docs.rs
│   ├── komandan/                 — the binary; package `komandan`
│   │   └── src/main.rs           — CLI entry; run_app() + plugin dispatch short-circuit
│   ├── komandan-plugin-abi/      — ABI-stable interface crate (abi_stable #[sabi_trait]);
│   │   └── src/                  — Plugin/CoreApi/LoggerSink traits, mirror types, entry symbol
│   ├── komandan-hello-plugin/    — reference cdylib plugin (libkomandan_hello_plugin.so)
│   └── komandan-playbook/        — playbook plugin (libkomandan_playbook.so): YAML parser + IR +
│                                   inventory + `--syntax-check`/`--list-hosts`/`--list-tasks`
│                                   (Phase 1), templating (Phase 2: `minijinja` engine +
│                                   `vars.rs` layered store + gap filters/tests/lookups + magic
│                                   vars + `omit`), **native execution** (Phase 3:
│                                   `executors/` ModuleExecutor registry + reuse/control/gap
│                                   shims, `connection_pool.rs`, `runner/` per-task flow,
│                                   `host.rs` ansible_* mapping), **and play orchestration**
│                                   (Phase 4: `runner/block.rs` block/rescue/always + loops +
│                                   handlers/notify (incl. `listen:`) + `serial:` batching +
│                                   `runner/tags.rs` + `runner/facts.rs` +
│                                   `--start-at-task`), **and roles/includes** (Phase 5:
│                                   `role.rs` loader + meta-dep topological sort +
│                                   `group_vars`/`host_vars` dir loading +
│                                   `include_tasks`/`import_tasks`). cdylib+rlib; depends on
│                                   komandan-plugin-abi + serde_yaml/indexmap/clap +
│                                   minijinja/regex/base64/bcrypt.
├── schema/komandan.schema.json   — JSON Schema for `komandan.json` (editor-time validation)
└── .cargo/config.toml            — link args (`-rdynamic`) for the cdylib plugin host
```

Built-in modules (`modules/core.rs`): `apt`, `cmd`, `dnf`, `download`, `file`,
`get_url`, `group`, `lineinfile`, `postgresql_user`, `script`,
`systemd_service`, `template`, `upload`, `user`.

Execution flow: `main → run_app → create_lua_with_args → run_main_file_with_args
→ (Lua) komandan.komando(task, host) → komando() → create_connection() →
SSHSession|LocalSession → module.run() → KomandoResult`.

### Plugin system (Phase 0)

- Plugins are cdylibs (`.so`/`.dylib`/`.dll`) loaded at startup via `libloading`
  from the plugin dir (see Commands). Each exports the C symbol
  `komandan_plugin_v1` (pinned by `komandan-plugin-abi::ENTRY_SYMBOL`).
- The interface is `abi_stable`-based: `komandan-plugin-abi` defines the
  `Plugin` (plugin-implemented), `CoreApi` (host-implemented), and `LoggerSink`
  traits as `#[sabi_trait]` objects, plus FFI-safe mirror types
  (`HostInfo`, `TaskInput`, `ModuleResult`, `RValue`, ...). Load-time layout
  checks guard against ABI drift.
- `CoreApi` is **fully wired except `worker_lua`/`now_playing_task`**
  (`plugin/core_api.rs::HostCore`, Phase 1):
  `create_connection` / `executor_run` / `executor_upload` /
  `executor_write_file` / `close_connection` drive a real connection registry
  (`Mutex<HashMap<u64, Connection>>`) over `CommandExecutor`; `defaults_get/set`,
  `report_record`, `global_flags`, `host_info`, `log` bridge to the live
  komandan-core surfaces. `komando` dispatches through the public Lua entrypoint
  `komandan.komando` on a **per-thread plugin Lua VM** (`PLUGIN_LUA`
  thread-local, lazily seeded via `create_lua()` — mirrors `komando.rs`'s
  rayon-worker `WORKER_LUA` pool but decoupled so the load-bearing parallel
  path stays untouched). `mlua::Lua` is `!Send`; this is sound because the VM
  lives in a `thread_local!` static (not a `HostCore` field), so `HostCore`
  stays `Send + Sync` and the VM never crosses threads. `worker_lua` /
  `now_playing_task` return v0.1 placeholders (closure-marshalling TBD).
  `plugin/conversions.rs` holds the `HostInfo`↔`Host`,
  `KomandoResult`↔`ModuleResult`, and recursive `RValue`→`mlua::Value` bridges
  (note: `Host`/`KomandoResult` fields are `pub(crate)` for this).
- Plugin dispatch: `komandan <plugin-name>` (the positional `main_file` arg) is
  matched against the loaded registry and, if found, short-circuits before Lua
  VM construction. Trailing-arg forwarding (`komandan hello --x y`) arrives with
  the external-subcommand routing planned for Phase 1.
- `--version` lists loaded plugins.

## Conventions

### Error handling
- `anyhow::Result` for application / fallible I/O code; `?` + `.context()`.
- `mlua::Result` for Lua-facing fns; convert with `mlua::Error::external`.
- Typed library errors via `thiserror` + `#[error("...")]`. The crate now has
  two minimal `thiserror` enums: `ParallelExecutorError` (single
  `Configuration` variant — executor tuning) and `ConnectionError` (five
  connection-establishment variants). Both flow into `anyhow::Error` via the
  blanket `From<E: StdError>`, so `?` works crate-wide with **no** manual
  `From` boilerplate. Do **not** reintroduce multi-`String`-field bespoke
  enums (the old `suggestion`/`troubleshooting`/`recovery_suggestion` pattern
  was deliberately removed in Phase 2). User-facing troubleshooting text
  belongs at call sites, not on error types.
- No `.unwrap()` / `.expect()` anywhere (denied by lint). No `panic!` in
  libraries.

### Lua interop
- Lua-facing fns take `(&Lua, (Value, Value))` or `(&Lua, Table)` and return
  `mlua::Result<Table>`.
- Use `mlua::chunk! { ... }` for inline Lua (captures Rust vars safely).
- Models implement `FromLua` / `IntoLua` (see `models.rs`).

### Performance — invariants to preserve
These were high-priority bugs; the fixes are now load-bearing. Do not regress:

- **`Args::parse()` lives only in `main.rs`.** It re-parses process argv every
  call. Config is threaded down from `main` via `create_lua_with_args(&Args)`
  / `run_main_file_with_args`. Never call `Args::parse()` from library or
  hot-path code.
- **One `Lua` VM per rayon worker, not per task.** `komando_parallel_*` obtain
  the VM from a `thread_local!` `OnceCell<Lua>` (lazily built once per worker
  via `create_lua()`). Never spawn a fresh VM inside a `par_iter` / `par_bridge`
  / `spawn` closure.
- **`unsafe` lives only in `crates/komandan-core/src/plugin/loader.rs`.** The
  workspace denies `unsafe_code`; that one module carries a file-scoped
  `#![allow(unsafe_code)]` for the `libloading` dlopen/dlsym that loads plugins.
  Do not add `unsafe` elsewhere in komandan-core. (The `#[sabi_trait]` macro
  expansion in `komandan-plugin-abi` emits its own `unsafe`, allowed narrowly
  inside that crate's `traits.rs` — that is macro-generated, not hand-written.)
  Plugin entry-symbol and `run()` calls are wrapped in `std::panic::catch_unwind`
  so a misbehaving plugin cannot abort the host.

### Style
- `cargo fmt` is authoritative.
- Doc comments (`///`) on every public item; include `# Errors` / `# Panics`
  where applicable. (Note: 0/14 module entry fns currently carry `///` docs —
  add docs when touching a module.)
- No commented-out code. No `TODO` without a linked issue.
- Secrets (passwords, passphrases) belong in `secrecy::SecretString`, not bare
  `String`. `Defaults` and `models::Host` both do this correctly now. Do not
  add new plaintext secret fields. Note: `Host` exposes secrets when building
  the Lua table for module consumption — that is the controlled boundary, not
  a leak; keep new exposure points deliberate.

### Tests
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each file.
  Co-located module-level tests also exist (e.g. `src/connection/tests.rs`,
  `src/parallel_executor/tests.rs`, `src/ssh/tests.rs`).
- Integration tests live in `tests/` and use `komandan::create_lua()` to build
  a real Lua VM.
- Integration tests that touch SSH to `localhost` require a local test user
  (`usertest`) with an authorized key — silently skipped on machines lacking
  it. Don't make them hard-fail.
- Every new module / public fn gets at least one test.

## Pull-request checklist

Before requesting review, confirm:

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — zero findings
- [ ] `cargo fmt --check` — clean
- [ ] `cargo test --workspace --all-targets` — no new failures (document any
      environment-dependent skips)
- [ ] No new `unwrap` / `expect` / `unsafe` / `panic!`
- [ ] Public items documented
- [ ] No secrets in logs or error strings
- [ ] Public API changes recorded in `CHANGELOG.md`
- [ ] Commit message follows Conventional Commits
      (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`)

## Known landmines (do not worsen)

- `ssh2` crate is a C-binding (libssh2). Pure-Rust `russh` migration is on the
  roadmap. New SSH features should be written against the `CommandExecutor`
  trait so the backend can be swapped without touching call sites.
- `mlua` is on a release candidate (`0.12.0-rc.2`). Pin and watch for breaking
  changes before bumping; do not pin to stable until `0.12.0` ships.
- `proc-macro-error2` is patched via `[patch.crates-io]` for a known
  future-incompat warning (E0365). Do not remove the patch until upstream
  releases; re-evaluate quarterly.
- `examples/gen_module_docs.rs` is pure-std and parses source heuristically —
  the `chunk!` bodies it scans hold implementation, not structured docstrings.
  Improve the generator alongside adding `///` to module entry fns.
- `abi_stable` is exact-pinned (`=0.11.3` in `[workspace.dependencies]`). The
  `#[sabi_trait]` macro bakes layout constants into generated code; a patch
  bump is safe, a minor bump needs deliberate review and an `ABI_VERSION` bump
  in `komandan-plugin-abi::entry`. Plugins must compile against the same
  abi_stable minor as the host or load-time layout checks reject them.
- `libloading` `Library::new` is `unsafe` (runs cdylib constructors). The only
  call site is `plugin/loader.rs`, audited; keep it there.
