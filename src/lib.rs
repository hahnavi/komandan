#![feature(once_cell_try)]

pub mod args;
mod checks;
pub mod connection;
pub mod defaults;
pub mod executor;
mod komando;
mod local;
pub mod models;
mod modules;
pub mod parallel_executor;
pub mod project;
mod repl_config;
mod report;
pub mod ssh;
mod util;
mod validator;

use anyhow::Result;
use args::Args;
use checks::collect_check_functions;
use defaults::Defaults;
use komando::{komando, komando_parallel_hosts, komando_parallel_tasks};
use mlua::{Lua, MultiValue, chunk};
use modules::{base_module, collect_core_modules};
use parallel_executor::{create_global_executor_interface, parallel_executor_constructor};
use report::generate_report;
use rustyline::DefaultEditor;
use std::{env, fs, path::Path};
use util::{
    dprint, filter_hosts, host_info, parse_hosts_json_file, parse_hosts_json_url, regex_is_match,
};

/// Cached `LuaJIT` version string, populated once on first `Lua` construction.
static LUAJIT_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Captures the `LuaJIT` version (`jit.version`, falling back to `_VERSION`)
/// into [`LUAJIT_VERSION`] the first time a `Lua` VM is built. Errors are
/// silently discarded — version capture must never break Lua creation.
fn capture_luajit_version(lua: &Lua) {
    let result = lua
        .load("return jit.version")
        .eval::<String>()
        .or_else(|_| lua.load("return _VERSION").eval::<String>());
    if let Ok(version) = result {
        let _ = LUAJIT_VERSION.set(version);
    }
}

/// Returns the `LuaJIT` version, populating the cache lazily.
///
/// On normal runs the version is captured during [`build_lua`]; this helper
/// covers the bare `--version` path, which prints and exits before any Komandan
/// `Lua` is constructed. In that case a throwaway `Lua::new()` (just the VM —
/// no Komandan table) is built once to read `jit.version`.
fn luajit_version() -> &'static str {
    LUAJIT_VERSION
        .get_or_init(|| {
            let lua = Lua::new();
            lua.load("return jit.version")
                .eval::<String>()
                .or_else(|_| lua.load("return _VERSION").eval::<String>())
                .unwrap_or_else(|_| "unknown".to_string())
        })
        .as_str()
}

/// Constructs a `Lua` VM, using the unsafe backend only when explicitly requested.
#[allow(unsafe_code)]
fn build_lua(unsafe_lua: bool) -> Lua {
    let lua = if unsafe_lua {
        // SAFETY: `unsafe_new` bypasses Lua's internal safety guarantees; only
        // enabled via the explicit `--unsafe-lua` CLI flag by a trusting user.
        unsafe { Lua::unsafe_new() }
    } else {
        Lua::new()
    };
    capture_luajit_version(&lua);
    lua
}

/// Prepends the project's Lua module search paths to `package.path`/`package.cpath`.
///
/// # Errors
///
/// Returns an error if loading/executing the package-path chunk fails.
fn configure_package_path(lua: &Lua, project_dir: &str) -> mlua::Result<()> {
    let project_dir_lua = project_dir;
    lua.load(chunk! {
        package.path = $project_dir_lua .. "/?.lua;" .. $project_dir_lua .. "/?;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?.lua;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?/init.lua;"  .. package.path
        package.cpath = $project_dir_lua .. "/?.so;" .. $project_dir_lua .. "/lua_modules/lib/lua/5.1/?.so;" .. package.cpath
    })
    .exec()?;
    Ok(())
}

/// Resolves the project directory from `args.main_file` (its parent dir),
/// falling back to the current working directory when unset or parent-less.
///
/// # Errors
///
/// Returns an error if the current working directory cannot be determined.
fn resolve_project_dir(args: &Args) -> mlua::Result<String> {
    match &args.main_file {
        Some(main_file) => {
            let parent = Path::new(main_file).parent();
            match parent {
                Some(p) if !p.display().to_string().is_empty() => Ok(p.display().to_string()),
                _ => Ok(env::current_dir()?.display().to_string()),
            }
        }
        None => Ok(env::current_dir()?.display().to_string()),
    }
}

/// Creates a new Lua instance with Komandan configuration.
///
/// Uses the already-initialized global config/flags (set by an earlier
/// `create_lua_with_args` call, or defaults when uninitialized). Prefer
/// `create_lua_with_args` when you have parsed `Args` in hand.
///
/// # Errors
///
/// Returns an error if Lua initialization or setup fails.
pub fn create_lua() -> mlua::Result<Lua> {
    let project_dir = crate::args::global_config().project_dir;
    let project_dir = if project_dir.is_empty() {
        env::current_dir()?.display().to_string()
    } else {
        project_dir
    };
    let lua = build_lua(crate::args::global_flags().unsafe_lua);
    configure_package_path(&lua, &project_dir)?;
    setup_komandan_table(&lua)?;
    Ok(lua)
}

/// Creates a new Lua instance with explicit arguments (avoids re-parsing CLI args).
///
/// Resolves the project directory from `args.main_file`, initializes the global
/// config, then builds the Lua VM and Komandan environment.
///
/// # Arguments
///
/// * `args` - Pre-parsed command-line arguments
///
/// # Errors
///
/// Returns an error if Lua initialization or setup fails.
pub fn create_lua_with_args(args: &Args) -> mlua::Result<Lua> {
    let project_dir = resolve_project_dir(args)?;

    crate::args::init_global_config(crate::args::ResolvedConfig {
        flags: args.flags.clone(),
        project_dir: project_dir.clone(),
    })
    .map_err(mlua::Error::external)?;

    let lua = build_lua(args.flags.unsafe_lua);
    configure_package_path(&lua, &project_dir)?;
    setup_komandan_table(&lua)?;
    Ok(lua)
}

/// Sets up the `komandan` global table in Lua.
///
/// # Errors
///
/// Returns an error if table creation or setting globals fails.
pub fn setup_komandan_table(lua: &Lua) -> mlua::Result<()> {
    let komandan = lua.create_table()?;
    komandan.set("defaults", Defaults::global())?;
    komandan.set("KomandanModule", base_module(lua)?)?;
    komandan.set("modules", collect_core_modules(lua)?)?;
    komandan.set("check", collect_check_functions(lua)?)?;
    komandan.set("parallel_executor", parallel_executor_constructor(lua)?)?;

    let entries = [
        ("komando", lua.create_function(komando)?),
        (
            "komando_parallel_tasks",
            lua.create_function(komando_parallel_tasks)?,
        ),
        (
            "komando_parallel_hosts",
            lua.create_function(komando_parallel_hosts)?,
        ),
        ("regex_is_match", lua.create_function(regex_is_match)?),
        ("filter_hosts", lua.create_function(filter_hosts)?),
        (
            "parse_hosts_json_file",
            lua.create_function(parse_hosts_json_file)?,
        ),
        (
            "parse_hosts_json_url",
            lua.create_function(parse_hosts_json_url)?,
        ),
        ("dprint", lua.create_function(dprint)?),
        ("host_info", lua.create_function(host_info)?),
    ];
    for (name, func) in &entries {
        komandan.set(*name, func.clone())?;
    }

    lua.globals().set("komandan", &komandan)?;

    let k_table = lua.create_table()?;
    k_table.set("defaults", komandan.get::<mlua::Value>("defaults")?)?;
    for (name, _) in &entries {
        k_table.set(*name, komandan.get::<mlua::Value>(*name)?)?;
    }
    k_table.set("mods", komandan.get::<mlua::Value>("modules")?)?;
    k_table.set("check", komandan.get::<mlua::Value>("check")?)?;
    k_table.set("parallel_executor", create_global_executor_interface(lua)?)?;
    lua.globals().set("k", k_table)?;

    Ok(())
}

/// Runs the main Lua file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or if Lua execution fails.
pub fn run_main_file(lua: &Lua, main_file: &String) -> Result<()> {
    let script = match fs::read_to_string(main_file) {
        Ok(script) => script,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to read the main file ({main_file}): {e}"
            ));
        }
    };

    lua.load(&script).set_name(main_file).exec()?;

    if !crate::args::global_flags().no_report {
        generate_report();
    }

    Ok(())
}

/// Runs the main Lua file with explicit arguments (avoids re-parsing CLI args).
///
/// This function is similar to `run_main_file()` but accepts explicit args to avoid
/// re-parsing command-line arguments. Use this when args have already been parsed.
///
/// # Arguments
///
/// * `lua` - The Lua context
/// * `args` - Pre-parsed command-line arguments
/// * `main_file` - Path to the main Lua file
///
/// # Errors
///
/// Returns an error if the file cannot be read or if Lua execution fails.
pub fn run_main_file_with_args(lua: &Lua, args: &Args, main_file: &String) -> Result<()> {
    let script = match fs::read_to_string(main_file) {
        Ok(script) => script,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to read the main file ({main_file}): {e}"
            ));
        }
    };

    lua.load(&script).set_name(main_file).exec()?;

    if !args.flags.no_report {
        generate_report();
    }

    Ok(())
}

/// Starts the REPL (Read-Eval-Print Loop).
///
/// # Errors
///
/// Returns an error if the editor cannot be initialized.
pub fn repl(lua: &Lua) -> Result<()> {
    print_version();
    let mut editor = DefaultEditor::with_config(repl_config::load_config())
        .map_err(|e| anyhow::anyhow!("Failed to create editor: {e}"))?;

    loop {
        let mut prompt = "> ";
        let mut line = String::new();

        loop {
            match editor.readline(prompt) {
                Ok(input) => line.push_str(&input),
                Err(_) => return Ok(()),
            }

            match lua.load(&line).eval::<MultiValue>() {
                Ok(values) => {
                    let _ = editor.add_history_entry(line);
                    println!(
                        "{}",
                        values
                            .iter()
                            .map(|value| format!("{value:#?}"))
                            .collect::<Vec<_>>()
                            .join("\t")
                    );
                    break;
                }
                Err(mlua::Error::SyntaxError {
                    incomplete_input: true,
                    ..
                }) => {
                    line.push('\n');
                    prompt = ">> ";
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    break;
                }
            }
        }
    }
}

pub fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let authors = env!("CARGO_PKG_AUTHORS");
    let sha = option_env!("KOMANDAN_GIT_SHA").unwrap_or("unknown");
    let arch = env::consts::ARCH;
    let os = env::consts::OS;
    let luajit = luajit_version();
    println!("Komandan {version} -- Copyright (C) 2026 {authors}");
    println!("git: {sha}  target: {arch}-{os}  luajit: {luajit}");
}

// Tests
#[cfg(test)]
mod tests {
    use clap::Parser;
    use mlua::Table;

    use super::*;

    #[test]
    fn test_cli_args() {
        let args = vec!["komandan", "--verbose", "/tmp/test/main.lua"];
        let args = Args::parse_from(args);
        assert_eq!(
            args,
            Args {
                chunk: None,
                main_file: Some("/tmp/test/main.lua".to_string()),
                command: None,
                flags: crate::args::Flags {
                    dry_run: false,
                    no_report: false,
                    interactive: false,
                    verbose: true,
                    unsafe_lua: false,
                    version: false,
                },
            }
        );
    }

    #[test]
    fn test_setup_komandan_table() -> Result<()> {
        let lua = create_lua()?;

        // Assert that the komandan table is set up correctly
        let komandan_table = lua.globals().get::<Table>("komandan")?;
        assert!(komandan_table.contains_key("defaults")?);
        assert!(komandan_table.contains_key("KomandanModule")?);
        assert!(komandan_table.contains_key("komando")?);
        assert!(komandan_table.contains_key("regex_is_match")?);
        assert!(komandan_table.contains_key("filter_hosts")?);
        assert!(komandan_table.contains_key("parse_hosts_json_file")?);
        assert!(komandan_table.contains_key("parse_hosts_json_url")?);
        assert!(komandan_table.contains_key("dprint")?);
        assert!(komandan_table.contains_key("host_info")?);

        let modules_table = komandan_table.get::<Table>("modules")?;
        assert!(modules_table.contains_key("apt")?);
        assert!(modules_table.contains_key("cmd")?);
        assert!(modules_table.contains_key("lineinfile")?);
        assert!(modules_table.contains_key("script")?);
        assert!(modules_table.contains_key("systemd_service")?);
        assert!(modules_table.contains_key("template")?);
        assert!(modules_table.contains_key("upload")?);
        assert!(modules_table.contains_key("download")?);

        // Test parallel executor constructor
        assert!(komandan_table.contains_key("parallel_executor")?);

        // Test check namespace
        let check_table = komandan_table.get::<Table>("check")?;
        assert!(check_table.contains_key("file")?);
        assert!(check_table.contains_key("service")?);
        assert!(check_table.contains_key("package")?);

        // Test aliases
        let k_table = lua.globals().get::<Table>("k")?;
        assert!(k_table.contains_key("defaults")?);
        assert!(k_table.contains_key("komando")?);
        assert!(k_table.contains_key("mods")?);
        assert!(k_table.contains_key("check")?);
        assert!(k_table.contains_key("parallel_executor")?);

        let k_mods_table = k_table.get::<Table>("mods")?;
        assert!(k_mods_table.contains_key("apt")?);
        assert!(k_mods_table.contains_key("cmd")?);

        let k_check_table = k_table.get::<Table>("check")?;
        assert!(k_check_table.contains_key("file")?);
        assert!(k_check_table.contains_key("service")?);
        assert!(k_check_table.contains_key("package")?);

        // Test global parallel executor interface
        let k_parallel_executor = k_table.get::<Table>("parallel_executor")?;
        assert!(k_parallel_executor.contains_key("map")?);
        assert!(k_parallel_executor.contains_key("configure")?);

        Ok(())
    }

    #[test]
    fn test_run_main_file() -> Result<()> {
        let lua = create_lua()?;

        // Test with a valid Lua file
        let main_file = "examples/hosts.lua".to_string();
        let result = run_main_file(&lua, &main_file);
        assert!(result.is_ok());

        Ok(())
    }
}
