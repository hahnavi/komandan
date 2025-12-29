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
mod report;
pub mod ssh;
mod util;
mod validator;

use anyhow::Result;
use args::Args;
use checks::collect_check_functions;
use clap::Parser;
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

/// Creates a new Lua instance with Komandan configuration.
///
/// This function initializes a Lua environment with all Komandan modules,
/// utilities, and the centralized connection factory for SSH and local connections.
///
/// # Errors
///
/// Returns an error if Lua initialization fails.
#[allow(unsafe_code)]
pub fn create_lua() -> mlua::Result<Lua> {
    let args = Args::parse();
    let lua = if args.flags.unsafe_lua {
        unsafe { Lua::unsafe_new() }
    } else {
        Lua::new()
    };

    let project_dir = match args.main_file {
        Some(main_file) => {
            let main_file_path = Path::new(&main_file);

            match main_file_path.parent() {
                Some(parent) => {
                    if parent.display().to_string() == "" {
                        env::current_dir()?.display().to_string()
                    } else {
                        parent.display().to_string()
                    }
                }
                _none => env::current_dir()?.display().to_string(),
            }
        }
        None => env::current_dir()?.display().to_string(),
    };

    let project_dir_lua = project_dir;
    lua.load(
        chunk! {
            package.path = $project_dir_lua .. "/?.lua;" .. $project_dir_lua .. "/?;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?.lua;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?/init.lua;"  .. package.path
            package.cpath = $project_dir_lua .. "/?.so;" .. $project_dir_lua .. "/lua_modules/lib/lua/5.1/?.so;" .. package.cpath
        }
    ).exec()?;

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

    let defaults = Defaults::global();
    komandan.set("defaults", defaults)?;

    let base_module = base_module(lua)?;
    komandan.set("KomandanModule", base_module)?;

    komandan.set("komando", lua.create_function(komando)?)?;
    komandan.set(
        "komando_parallel_tasks",
        lua.create_function(komando_parallel_tasks)?,
    )?;
    komandan.set(
        "komando_parallel_hosts",
        lua.create_function(komando_parallel_hosts)?,
    )?;

    // Add utils
    komandan.set("regex_is_match", lua.create_function(regex_is_match)?)?;
    komandan.set("filter_hosts", lua.create_function(filter_hosts)?)?;
    komandan.set(
        "parse_hosts_json_file",
        lua.create_function(parse_hosts_json_file)?,
    )?;
    komandan.set(
        "parse_hosts_json_url",
        lua.create_function(parse_hosts_json_url)?,
    )?;
    komandan.set("dprint", lua.create_function(dprint)?)?;
    komandan.set("host_info", lua.create_function(host_info)?)?;

    // Add core modules
    komandan.set("modules", collect_core_modules(lua)?)?;

    // Add check functions
    komandan.set("check", collect_check_functions(lua)?)?;

    // Add parallel executor constructor
    komandan.set("parallel_executor", parallel_executor_constructor(lua)?)?;

    lua.globals().set("komandan", &komandan)?;

    // Create separate 'k' table (not an alias)
    let k_table = lua.create_table()?;

    // Copy core functionality to k
    k_table.set("defaults", komandan.get::<mlua::Value>("defaults")?)?;
    k_table.set("komando", komandan.get::<mlua::Value>("komando")?)?;
    k_table.set(
        "komando_parallel_hosts",
        komandan.get::<mlua::Value>("komando_parallel_hosts")?,
    )?;
    k_table.set(
        "komando_parallel_tasks",
        komandan.get::<mlua::Value>("komando_parallel_tasks")?,
    )?;

    // Copy utility functions
    k_table.set(
        "regex_is_match",
        komandan.get::<mlua::Value>("regex_is_match")?,
    )?;
    k_table.set("filter_hosts", komandan.get::<mlua::Value>("filter_hosts")?)?;
    k_table.set(
        "parse_hosts_json_file",
        komandan.get::<mlua::Value>("parse_hosts_json_file")?,
    )?;
    k_table.set(
        "parse_hosts_json_url",
        komandan.get::<mlua::Value>("parse_hosts_json_url")?,
    )?;
    k_table.set("dprint", komandan.get::<mlua::Value>("dprint")?)?;
    k_table.set("host_info", komandan.get::<mlua::Value>("host_info")?)?;

    lua.globals().set("k", k_table)?;

    // Create alias 'k.mods' for 'komandan.modules'
    let k_table = lua.globals().get::<mlua::Table>("k")?;
    let modules_table = komandan.get::<mlua::Table>("modules")?;
    k_table.set("mods", modules_table)?;

    // Create alias 'k.check' for 'komandan.check'
    let check_table = komandan.get::<mlua::Table>("check")?;
    k_table.set("check", check_table)?;

    // Add global parallel executor interface to k
    k_table.set("parallel_executor", create_global_executor_interface(lua)?)?;

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

    if !Args::parse().flags.no_report {
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
    let mut editor =
        DefaultEditor::new().map_err(|e| anyhow::anyhow!("Failed to create editor: {e}"))?;

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
    println!("Komandan {version} -- Copyright (C) 2025 {authors}");
}

// Tests
#[cfg(test)]
mod tests {
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
