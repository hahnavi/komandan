# AGENTS.md

Guide for AI agents (and humans) working on Komandan. Read this before editing.

## What is Komandan

Agentless server-automation tool. Embeds LuaJIT (via `mlua`) as the scripting
layer; Rust provides the runtime — connection factory, modules, parallel
executor, checks. Users write `*.lua` task scripts; Rust executes them over SSH
or locally. Think "Ansible, but Lua-scripted and written in Rust".

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
cargo test --lib                          # unit tests only (fast: ~3s)
```

`tests/check_system_integration.rs::test_package_validation_with_real_packages`
is environment-dependent (needs a real package manager + specific packages).
It may fail on stripped CI runners — that is pre-existing, not a regression.

## Architecture

```
src/
├── main.rs            — CLI entry, run_app() orchestrates subcommand vs run
├── lib.rs             — create_lua() / setup_komandan_table() / REPL
├── args.rs            — clap CLI definition (Args, Flags, Commands)
├── models.rs          — Host, Task, Module, KomandoResult, KomandanConfig
├── executor.rs        — CommandExecutor trait (impl by SSHSession + LocalSession)
├── connection/mod.rs  — create_connection() factory + SSH/local selection
├── ssh.rs             — SSHSession (wraps libssh2 via `ssh2` crate)
├── local.rs           — LocalSession (shells out locally)
├── komando.rs         — komando() / komando_parallel_{tasks,hosts}() core fns
├── parallel_executor.rs — generic map/each/reduce parallel framework (rayon)
├── defaults.rs        — global Defaults singleton (OnceLock + RwLock)
├── modules/           — one file per built-in module (cmd, apt, file, ...)
│   ├── base.rs        — Lua-side KomandanModule prototype
│   └── core.rs        — collect_core_modules() registers all modules
├── checks/            — system validation (file/package/service)
├── validator.rs       — Lua-table validators for host & task
├── report.rs          — execution report accumulator
├── project.rs         — `project init` / `project new` scaffolding
└── util.rs            — host_info, filter_hosts, parse_hosts_json_*, dprint, ...
```

Execution flow: `main → run_app → create_lua_with_args → run_main_file_with_args
→ (Lua) komandan.komando(task, host) → komando() → create_connection() →
SSHSession|LocalSession → module.run() → KomandoResult`.

## Conventions

### Error handling
- Use `anyhow::Result` for application/fallible I/O code; `?` + `.context()`.
- Use `mlua::Result` for Lua-facing functions; convert with `mlua::Error::external`.
- Do NOT add new bespoke error enums stuffed with `String` "suggestion" /
  "troubleshooting" fields — that pattern already exists in
  `parallel_executor.rs` and `connection/mod.rs` and is slated for removal
  (see REFACTOR_PLAN.md §3). Prefer `thiserror` + `#[error("...")]` for
  library-style typed errors.
- No `.unwrap()` / `.expect()` anywhere (denied by lint). No `panic!` in
  libraries.

### Lua interop
- Lua-facing fns take `(&Lua, (Value, Value))` or `(&Lua, Table)` and return
  `mlua::Result<Table>`.
- Use `mlua::chunk! { ... }` for inline Lua (captures Rust vars safely).
- Models implement `FromLua` / `IntoLua` (see `models.rs`).

### Performance — critical rules
- **Never call `Args::parse()` from library/hot-path code.** It re-parses the
  process argv every call. `komando.rs`, `util.rs::dprint`, and `report.rs`
  currently do this — it is tracked as a high-priority bug
  (REFACTOR_PLAN.md §1). New code must thread `&Args` (or a resolved config)
  down from `main`.
- **Never spawn a fresh `Lua` VM per task.** `create_lua()` builds the full
  Komandan table every call (~ms). `komando_parallel_*` currently call it
  inside `par_iter` — this is the #1 perf bug (REFACTOR_PLAN.md §2).

### Style
- `cargo fmt` is authoritative.
- Doc comments (`///`) on every public item; include `# Errors` / `# Panics`
  sections where applicable.
- No commented-out code. No `TODO` without a linked issue.
- Secrets (passwords, passphrases) belong in `secrecy::SecretString`, not bare
  `String`. `Defaults` does this correctly; `models::Host` does not yet —
  don't add new plaintext secret fields.

### Tests
- Unit tests live in `#[cfg(test)] mod tests` at the bottom of each file.
- Integration tests live in `tests/` and use `komandan::create_lua()` to build
  a real Lua VM.
- Integration tests that touch SSH to `localhost` require a local test user
  (`usertest`) with an authorized key — they are silently skipped on machines
  that lack it. Don't make them hard-fail.
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
- [ ] Commit message follows Conventional Commits
      (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`)

## Known landmines (do not worsen)

- `ssh2` crate is a C-binding (libssh2). Pure-Rust `russh` migration is on the
  roadmap. New SSH features should be written against the `CommandExecutor`
  trait so the backend can be swapped without touching call sites.
- `mlua` is on a release candidate (`0.12.0-rc.2`). Pin and watch for breaking
  changes before bumping.
- `proc-macro-error2` is patched via `[patch.crates-io]` for a known
  future-incompat warning. Do not remove the patch until upstream releases.
