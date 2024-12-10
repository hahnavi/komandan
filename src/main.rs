use args::Args;
use clap::Parser;
use mlua::{chunk, Error::RuntimeError, Integer, MultiValue, Lua, Table, Value};
use modules::{cmd, script, upload, download};
use rustyline::DefaultEditor;
use ssh::SSHSession;
use std::{env, path::Path};

mod args;
mod defaults;
mod modules;
mod ssh;
mod util;
mod validator;

use util::{dprint, filter_hosts, regex_is_match};
use validator::{validate_host, validate_module, validate_task};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    let lua = Lua::new();

    setup_komandan_table(&lua)?;

    let chunk = args.chunk.clone();
    match chunk {
        Some(chunk) => {
            lua.load(&chunk).eval::<()>()?;
        }
        None => {},
    }

    match &args.main_file {
        Some(main_file) => {
            run_main_file(&lua, main_file)?;
        },
        None => {
            if args.chunk.is_none() {
                repl(&lua);
            }
        },
    };

    if args.interactive && (!&args.main_file.is_none() || !&args.chunk.is_none()) {
        repl(&lua);
    }

    Ok(())
}

fn setup_komandan_table(lua: &Lua) -> mlua::Result<()> {
    let komandan = lua.create_table()?;

    komandan.set("defaults", defaults::defaults(&lua)?)?;

    let komandan_module = lua
        .load(chunk! {
                local KomandanModule = {}

        function KomandanModule:new(data)
            local o = setmetatable({}, { __index = self })
            o.name = data.name
            o.script = data.script
            return o
        end

        function KomandanModule:run_module()
        self:run()
        end

        return KomandanModule
            })
        .eval::<Table>()?;

    komandan.set("KomandanModule", komandan_module)?;

    komandan.set("set_defaults", lua.create_function(set_defaults)?)?;
    komandan.set("komando", lua.create_async_function(komando)?)?;

    // Add utils
    komandan.set("regex_is_match", lua.create_function(regex_is_match)?)?;
    komandan.set("filter_hosts", lua.create_function(filter_hosts)?)?;
    komandan.set("dprint", lua.create_function(dprint)?)?;

    // Add core modules
    let modules_table = lua.create_table()?;
    modules_table.set("cmd", lua.create_function(cmd)?)?;
    modules_table.set("script", lua.create_function(script)?)?;
    modules_table.set("upload", lua.create_function(upload)?)?;
    modules_table.set("download", lua.create_function(download)?)?;
    komandan.set("modules", modules_table)?;

    lua.globals().set("komandan", &komandan)?;

    Ok(())
}

fn set_defaults(lua: &Lua, data: Value) -> mlua::Result<()> {
    if !data.is_table() {
        return Err(RuntimeError(
            "Parameter for set_defaults must be a table.".to_string(),
        ));
    }

    let defaults = lua
        .globals()
        .get::<Table>("komandan")?
        .get::<Table>("defaults")?;

    for pair in data.as_table().unwrap().pairs() {
        let (key, value): (String, Value) = pair?;
        defaults.set(key, value.clone())?;
    }

    Ok(())
}

async fn komando(lua: Lua, (host, task): (Value, Value)) -> mlua::Result<Table> {
    let host = lua.create_function(validate_host)?.call::<Table>(&host)?;
    let task = lua.create_function(validate_task)?.call::<Table>(&task)?;
    let module = lua
        .create_function(validate_module)?
        .call::<Table>(task.get::<Table>(1).unwrap())?;

    let hostname_display = hostname_display(&host);

    let ssh = match SSHSession::connect(&lua, &host) {
        Ok(ssh) => ssh,
        Err(err) => {
            return Err(RuntimeError(format!(
                "Failed to connect to host '{}': {}",
                &hostname_display,
                &err.to_string()
            )))
        }
    };

    let task_name = match task.get::<String>("name") {
        Ok(name) => name,
        Err(_) => module.get::<String>("name")?,
    };

    let module_clone = module.clone();
    let results = lua
        .load(chunk! {
            print("Running task '" .. $task_name .. "' on host '" .. $hostname_display .."' ...")
            $module_clone.ssh = $ssh
            $module_clone:run()

            local results = $module_clone.ssh:get_session_results()
            komandan.dprint(results.stdout)
            if results.exit_code ~= 0 then
                print("Task '" .. $task_name .. "' on host '" .. $hostname_display .."' failed with exit code " .. results.exit_code .. ": " .. results.stderr)
            else
                print("Task '" .. $task_name .. "' on host '" .. $hostname_display .."' succeeded.")
            end

            return results
        })
        .eval::<Table>()?;

    lua.load(chunk! {
        if $module.cleanup ~= nil then
        $module:cleanup()
    end
    })
    .eval::<()>()?;

    let defaults = lua
        .globals()
        .get::<Table>("komandan")?
        .get::<Table>("defaults")?;

    let ignore_exit_code = task
        .get::<bool>("ignore_exit_code")
        .unwrap_or_else(|_| defaults.get::<bool>("ignore_exit_code").unwrap());

    if results.get::<Integer>("exit_code").unwrap() != 0 && !ignore_exit_code {
        return Err(RuntimeError("Failed to run task.".to_string()));
    }

    Ok(results)
}

fn hostname_display(host: &Table) -> String {
    let address = host.get::<String>("address").unwrap();

    match host.get::<String>("name") {
        Ok(name) => format!("{} ({})", name, address),
        Err(_) => format!("{}", address),
    }
}

fn repl(lua: &Lua) {
    print_version();
    let mut editor = DefaultEditor::new().expect("Failed to create editor");

    loop {
        let mut prompt = "> ";
        let mut line = String::new();

        loop {
            match editor.readline(prompt) {
                Ok(input) => line.push_str(&input),
                Err(_) => return,
            }

            match lua.load(&line).eval::<MultiValue>() {
                Ok(values) => {
                    editor.add_history_entry(line).unwrap();
                    println!(
                        "{}",
                        values
                            .iter()
                            .map(|value| format!("{:#?}", value))
                            .collect::<Vec<_>>()
                            .join("\t")
                    );
                    break;
                }
                Err(mlua::Error::SyntaxError {
                    incomplete_input: true,
                    ..
                }) => {
                    line.push_str("\n");
                    prompt = ">> ";
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    break;
                }
            }
        }
    }
}

fn run_main_file(lua: &Lua, main_file: &String) -> anyhow::Result<()> {
    let main_file_path = Path::new(&main_file);
    let main_file_name = main_file_path
        .file_stem()
        .unwrap_or_default()
        .to_str()
        .unwrap()
        .to_string();

    let main_file_ext = main_file_path
        .extension()
        .unwrap_or_default()
        .to_str()
        .unwrap()
        .to_string();

    if main_file_ext != "lua" {
        return Err(anyhow::anyhow!("Main file must be a lua file."));
    }

    let project_dir = match main_file_path.parent() {
        Some(parent) => Some(
            parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf()),
        ),
        _none => None,
    }
    .unwrap()
    .display()
    .to_string();

    let project_dir_lua = lua.create_string(&project_dir)?;
    lua.load(
        chunk! {
            local project_dir = $project_dir_lua
            package.path = project_dir .. "/?.lua;" .. project_dir .. "/lua_modules/share/lua/5.1/?.lua;" .. project_dir .. "/lua_modules/share/lua/5.1/?/init.lua;"
            package.cpath = project_dir .. "/?.so;" .. project_dir .. "/lua_modules/lib/lua/5.1/?.so;"
        }
    ).exec()?;

    lua.load(chunk! {
        require($main_file_name)
    })
    .set_name("main")
    .exec()?;

    Ok(())
}

fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let authors = env!("CARGO_PKG_AUTHORS");
    println!("Komandan {} -- Copyright (C) 2024 {}", version, authors);
}
