# AGENTS.md

Guide for AI agents (and humans) working on Komandan. Read before editing.

## What is Komandan

Agentless server-automation tool. Embeds LuaJIT (via `mlua`) as the scripting
layer; Rust provides the runtime — connection factory, 14 built-in modules,
parallel executor, system checks. Users write `*.lua` task scripts; Rust
executes them over SSH or locally. Think "Ansible, but Lua-scripted and
written in Rust". Current version `0.0.4`.

Status: Phases 0–2 of `REFACTOR_PLAN.md` are landed (perf hot spots + error
model). Phases 3+ remain. Consult that file for what is done vs. open.

## Toolchain

- **Rust nightly**, edition 2024. Pinned via `rust-toolchain.toml`.
- `clippy.toml` sets `too-many-lines-threshold = 500`.
- `Cargo.toml` denies: `unsafe_code`, `clippy::pedantic`, `clippy::nursery`,
  `clippy::unwrap_used`, `clippy::expect_used`, `clippy::enum_glob_use`.
- Any new code MUST compile under `cargo clippy --all-targets -- -D warnings`
  with zero findings.

## Commands

```bash
cargo build --all-targets                 # compile (first build ~2-3 min: builds LuaJIT + libssh2)
cargo clippy --all-targets -- -D warnings # lint gate — MUST be clean
cargo fmt                                 # format; --check in CI
cargo test --all-targets                  # full suite (lib + integration)
cargo test --lib                          # unit tests only (fast)
```

`tests/check_system_integration.rs::test_package_validation_with_real_packages`
is environment-dependent (needs a real package manager + specific packages).
It may fail on stripped CI runners — pre-existing, not a regression.

## Architecture

```
src/
├── main.rs              — CLI entry; run_app() orchestrates subcommand vs run
├── lib.rs               — create_lua[_with_args]() / setup_komandan_table() / REPL
├── args.rs              — clap CLI definition (Args, Flags, Commands)
├── models.rs            — Host, Task, Module, KomandoResult, KomandanConfig
├── executor.rs          — CommandExecutor trait (impl by SSHSession + LocalSession)
├── komando.rs           — komando() + komando_parallel_{tasks,hosts}(); worker Lua
│                          via thread_local! OnceCell<Lua> (one VM per rayon worker)
├── local.rs             — LocalSession (shells out locally)
├── ssh.rs               — SSHSession (wraps libssh2 via `ssh2` crate); module root for ssh/
├── ssh/                 — SSH submodules: auth, elevation, env, error, session, tests
├── connection/          — create_connection() factory + SSH/local selection, tests
├── parallel_executor/   — generic map/each/reduce framework (rayon): mod, batch,
│                          config, error, lua_bridge, monitor, pool, summary,
│                          validation, tests
├── defaults.rs          — global Defaults singleton (OnceLock + RwLock)
├── modules/             — one file per built-in module (see list below) + base.rs
│   └── core.rs          — collect_core_modules() registers all 14 modules
├── checks/              — system validation: base/, file/, package/, service/
├── validator.rs         — Lua-table validators for host & task
├── report.rs            — execution report accumulator
├── project.rs           — `project init` / `project new` scaffolding
├── repl_config.rs       — REPL config from komandan/repl.conf (rustyline settings)
├── templates/           — scaffolding assets: hosts.lua, main.lua, komandan.json.j2
└── util/                — host_info, filter_hosts, parse_hosts_json_*, dprint, ...
```

Built-in modules (`modules/core.rs`): `apt`, `cmd`, `dnf`, `download`, `file`,
`get_url`, `group`, `lineinfile`, `postgresql_user`, `script`,
`systemd_service`, `template`, `upload`, `user`.

Auxiliary trees:
- `examples/` — `.lua` task samples + `gen_module_docs.rs` (regenerates
  `docs/modules.md` by parsing `core.rs` + per-module doc comments).
- `docs/modules.md` — generated Lua module reference.
- `schema/komandan.schema.json` — JSON Schema for `komandan.json` (editor-time
  validation; no runtime validator dep — serde structurally validates).
- `build.rs` — embeds `git describe` + target for `--version`.

Execution flow: `main → run_app → create_lua_with_args → run_main_file_with_args
→ (Lua) komandan.komando(task, host) → komando() → create_connection() →
SSHSession|LocalSession → module.run() → KomandoResult`.

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

### Style
- `cargo fmt` is authoritative.
- Doc comments (`///`) on every public item; include `# Errors` / `# Panics`
  where applicable. (Note: 0/14 module entry fns currently carry `///` docs,
  so `docs/modules.md` renders "(no description)" for them — add docs when
  touching a module.)
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

- [ ] `cargo clippy --all-targets -- -D warnings` — zero findings
- [ ] `cargo fmt --check` — clean
- [ ] `cargo test --all-targets` — no new failures (document any
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
