mod args;
mod defaults;
mod modules;
pub mod ssh;
mod util;
mod validator;

use args::Args;
use clap::Parser;
use mlua::{chunk, Error::RuntimeError, Integer, Lua, MultiValue, Table, Value};
use modules::{apt, base_module, cmd, download, lineinfile, script, template, upload};
use rustyline::DefaultEditor;
use ssh::{ElevateMethod, Elevation, SSHAuthMethod, SSHSession};
use std::{env, fs, path::Path};
use util::{
    dprint, filter_hosts, host_display, parse_hosts_json, regex_is_match, set_defaults,
    task_display,
};
use validator::{validate_host, validate_task};

pub fn setup_lua_env(lua: &Lua) -> mlua::Result<()> {
    let args = Args::parse();

    let project_dir = match args.main_file.clone() {
        Some(main_file) => {
            let main_file_path = Path::new(&main_file);
            let project_dir = match main_file_path.parent() {
                Some(parent) => Some(
                    parent
                        .canonicalize()
                        .unwrap_or_else(|_| parent.to_path_buf()),
                ),
                _none => None,
            }
            .unwrap();
            project_dir.display().to_string()
        }
        None => env::current_dir()?.display().to_string(),
    };

    let project_dir_lua = project_dir.clone();
    lua.load(
        chunk! {
            package.path = $project_dir_lua .. "/?.lua;" .. $project_dir_lua .. "/?;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?.lua;" .. $project_dir_lua .. "/lua_modules/share/lua/5.1/?/init.lua;"  .. package.path
            package.cpath = $project_dir_lua .. "/?.so;" .. $project_dir_lua .. "/lua_modules/lib/lua/5.1/?.so;" .. package.cpath
        }
    ).exec()?;

    setup_komandan_table(&lua)?;

    Ok(())
}

pub fn setup_komandan_table(lua: &Lua) -> mlua::Result<()> {
    let komandan = lua.create_table()?;

    komandan.set("defaults", defaults::defaults(&lua)?)?;

    let base_module = base_module(&lua);
    komandan.set("KomandanModule", base_module)?;

    komandan.set("set_defaults", lua.create_function(set_defaults)?)?;
    komandan.set("komando", lua.create_function(komando)?)?;

    // Add utils
    komandan.set("regex_is_match", lua.create_function(regex_is_match)?)?;
    komandan.set("filter_hosts", lua.create_function(filter_hosts)?)?;
    komandan.set("parse_hosts_json", lua.create_function(parse_hosts_json)?)?;
    komandan.set("dprint", lua.create_function(dprint)?)?;

    // Add core modules
    let modules_table = lua.create_table()?;
    modules_table.set("apt", lua.create_function(apt)?)?;
    modules_table.set("cmd", lua.create_function(cmd)?)?;
    modules_table.set("lineinfile", lua.create_function(lineinfile)?)?;
    modules_table.set("script", lua.create_function(script)?)?;
    modules_table.set("template", lua.create_function(template)?)?;
    modules_table.set("upload", lua.create_function(upload)?)?;
    modules_table.set("download", lua.create_function(download)?)?;
    komandan.set("modules", modules_table)?;

    lua.globals().set("komandan", &komandan)?;

    Ok(())
}

fn komando(lua: &Lua, (host, task): (Value, Value)) -> mlua::Result<Table> {
    let host = lua.create_function(validate_host)?.call::<Table>(&host)?;
    let task = lua.create_function(validate_task)?.call::<Table>(&task)?;
    let module = task.get::<Table>(1)?;

    let host_display = host_display(&host);
    let task_display = task_display(&task);

    let defaults = lua
        .globals()
        .get::<Table>("komandan")?
        .get::<Table>("defaults")?;

    let user = match host.get::<String>("user") {
        Ok(user) => user,
        Err(_) => match defaults.get::<String>("user") {
            Ok(user) => user,
            Err(_) => match env::var("USER") {
                Ok(user) => user,
                Err(_) => {
                    return Err(RuntimeError(format!(
                        "No user specified for task '{}'.",
                        task_display
                    )))
                }
            },
        },
    };

    let ssh_auth_method = match host.get::<String>("private_key_file") {
        Ok(private_key_file) => SSHAuthMethod::PublicKey {
            private_key: private_key_file,
            passphrase: match host.get::<String>("private_key_pass") {
                Ok(passphrase) => Some(passphrase),
                Err(_) => match defaults.get::<String>("private_key_pass") {
                    Ok(passphrase) => Some(passphrase),
                    Err(_) => None,
                },
            },
        },
        Err(_) => match defaults.get::<String>("private_key_file") {
            Ok(private_key_file) => SSHAuthMethod::PublicKey {
                private_key: private_key_file,
                passphrase: match host.get::<String>("private_key_pass") {
                    Ok(passphrase) => Some(passphrase),
                    Err(_) => match defaults.get::<String>("private_key_pass") {
                        Ok(passphrase) => Some(passphrase),
                        Err(_) => None,
                    },
                },
            },
            Err(_) => match host.get::<String>("password") {
                Ok(password) => SSHAuthMethod::Password(password),
                Err(_) => match defaults.get::<String>("password") {
                    Ok(password) => SSHAuthMethod::Password(password),
                    Err(_) => {
                        return Err(RuntimeError(format!(
                            "No authentication method specified for task '{}' on host '{}'.",
                            task_display, host_display
                        )))
                    }
                },
            },
        },
    };

    let port = host.get::<Integer>("port").unwrap_or_else(|_| {
        defaults
            .get::<Integer>("port")
            .unwrap_or_else(|_| 22.into())
    }) as u16;

    let elevate = match task.get::<Value>("elevate") {
        Ok(elevate) => match elevate {
            Value::Nil => match host.get::<Value>("elevate") {
                Ok(elevate) => match elevate {
                    Value::Nil => match defaults.get::<Value>("elevate") {
                        Ok(elevate) => match elevate {
                            Value::Nil => false,
                            Value::Boolean(true) => true,
                            _ => false,
                        },
                        Err(_) => false,
                    },
                    Value::Boolean(true) => true,
                    _ => false,
                },
                Err(_) => false,
            },
            Value::Boolean(true) => true,
            _ => false,
        },
        Err(_) => false,
    };

    let as_user: Option<String> = match task.get::<String>("as_user") {
        Ok(as_user) => Some(as_user),
        Err(_) => match host.get::<String>("as_user") {
            Ok(as_user) => Some(as_user),
            Err(_) => match defaults.get::<String>("as_user") {
                Ok(as_user) => Some(as_user),
                Err(_) => None,
            },
        },
    };

    let elevation_method_str = match elevate {
        true => match user.as_str() {
            "root" => "su".to_string(),
            _ => match task.get::<String>("elevation_method") {
                Ok(method) => method,
                Err(_) => match host.get::<String>("elevation_method") {
                    Ok(method) => method,
                    Err(_) => match defaults.get::<String>("elevation_method") {
                        Ok(method) => method,
                        Err(_) => "none".to_string(),
                    },
                },
            },
        },
        false => "none".to_string(),
    };

    let elevation_method = match elevation_method_str.to_lowercase().as_str() {
        "none" => ElevateMethod::None,
        "su" => ElevateMethod::Su,
        "sudo" => ElevateMethod::Sudo,
        _ => {
            return Err(RuntimeError(format!(
                "Invalid elevation_method '{}' for task '{}' on host '{}'.",
                elevation_method_str, task_display, host_display
            )))
        }
    };

    let elevation = Elevation {
        method: elevation_method,
        as_user,
    };

    let known_hosts_file = match host.get::<String>("known_hosts_file") {
        Ok(known_hosts_file) => Some(known_hosts_file),
        Err(_) => match defaults.get::<String>("known_hosts_file") {
            Ok(known_hosts_file) => Some(known_hosts_file),
            Err(_) => None,
        },
    };

    let host_key_check = match host.get::<Value>("host_key_check") {
        Ok(host_key_check) => match host_key_check {
            Value::Nil => match defaults.get::<Value>("host_key_check") {
                Ok(host_key_check) => match host_key_check {
                    Value::Nil => true,
                    Value::Boolean(false) => false,
                    _ => true,
                },
                Err(_) => true,
            },
            Value::Boolean(false) => false,
            _ => true,
        },
        Err(_) => true,
    };

    let mut ssh = SSHSession::new()?;

    if host_key_check {
        ssh.known_hosts_file = known_hosts_file;
    }

    ssh.elevation = elevation;

    ssh.connect(
        host.get::<String>("address")?.as_str(),
        port,
        &user,
        ssh_auth_method,
    )?;

    let env_defaults = defaults.get::<Table>("env").unwrap_or(lua.create_table()?);
    let env_host = host.get::<Table>("env").unwrap_or(lua.create_table()?);
    let env_task = task.get::<Table>("env").unwrap_or(lua.create_table()?);

    for pair in env_defaults.pairs() {
        let (key, value): (String, String) = pair?;
        ssh.set_env(&key, &value);
    }

    for pair in env_host.pairs() {
        let (key, value): (String, String) = pair?;
        ssh.set_env(&key, &value);
    }

    for pair in env_task.pairs() {
        let (key, value): (String, String) = pair?;
        ssh.set_env(&key, &value);
    }

    let results = lua
        .load(chunk! {
            print("Running task '" .. $task_display .. "' on host '" .. $host_display .."' ...")
            $module.ssh = $ssh
            $module:run()

            local results = $module.ssh:get_session_results()
            komandan.dprint(results.stdout)
            if results.exit_code ~= 0 then
                print("Task '" .. $task_display .. "' on host '" .. $host_display .."' failed with exit code " .. results.exit_code .. ": " .. results.stderr)
            else
                print("Task '" .. $task_display .. "' on host '" .. $host_display .."' succeeded.")
            end

            if $module.cleanup ~= nil then
                $module:cleanup()
            end

            return results
        })
        .eval::<Table>()?;

    let ignore_exit_code = task
        .get::<bool>("ignore_exit_code")
        .unwrap_or_else(|_| defaults.get::<bool>("ignore_exit_code").unwrap());

    if results.get::<Integer>("exit_code").unwrap() != 0 && !ignore_exit_code {
        return Err(RuntimeError("Failed to run task.".to_string()));
    }

    Ok(results)
}

pub fn run_main_file(lua: &Lua, main_file: &String) -> anyhow::Result<()> {
    let script = match fs::read_to_string(main_file) {
        Ok(script) => script,
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to read the main file ({}): {}", main_file, e));
        }
    };

    lua.load(&script).set_name(main_file).exec()?;

    Ok(())
}

pub fn repl(lua: &Lua) {
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

pub fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let authors = env!("CARGO_PKG_AUTHORS");
    println!("Komandan {} -- Copyright (C) 2024 {}", version, authors);
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_args() {
        let args = vec!["komandan", "--verbose", "/tmp/test/main.lua"];
        let args = Args::parse_from(args);
        assert_eq!(
            args,
            Args {
                chunk: None,
                interactive: false,
                verbose: true,
                version: false,
                main_file: Some("/tmp/test/main.lua".to_string()),
            }
        );
    }

    #[test]
    fn test_setup_komandan_table() {
        let lua = Lua::new();
        setup_komandan_table(&lua).unwrap();

        // Assert that the komandan table is set up correctly
        let komandan_table = lua.globals().get::<Table>("komandan").unwrap();
        assert!(komandan_table.contains_key("defaults").unwrap());
        assert!(komandan_table.contains_key("KomandanModule").unwrap());
        assert!(komandan_table.contains_key("set_defaults").unwrap());
        assert!(komandan_table.contains_key("komando").unwrap());
        assert!(komandan_table.contains_key("regex_is_match").unwrap());
        assert!(komandan_table.contains_key("filter_hosts").unwrap());
        assert!(komandan_table.contains_key("parse_hosts_json").unwrap());
        assert!(komandan_table.contains_key("dprint").unwrap());

        let modules_table = komandan_table.get::<Table>("modules").unwrap();
        assert!(modules_table.contains_key("apt").unwrap());
        assert!(modules_table.contains_key("cmd").unwrap());
        assert!(modules_table.contains_key("lineinfile").unwrap());
        assert!(modules_table.contains_key("script").unwrap());
        assert!(modules_table.contains_key("template").unwrap());
        assert!(modules_table.contains_key("upload").unwrap());
        assert!(modules_table.contains_key("download").unwrap());
    }

    #[test]
    fn test_run_main_file() {
        let lua = Lua::new();

        // Test with a valid Lua file
        let main_file = "examples/hosts.lua".to_string();
        let result = run_main_file(&lua, &main_file);
        assert!(result.is_ok());
    }
}
