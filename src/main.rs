use args::Args;
use clap::Parser;
use mlua::{chunk, Error::RuntimeError, Integer, Lua, MultiValue, Table, Value};
use modules::{apt, base_module, cmd, download, script, upload};
use rustyline::DefaultEditor;
use ssh::{ElevateMethod, Elevation, SSHAuthMethod, SSHSession};
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
        None => {}
    }

    match &args.main_file {
        Some(main_file) => {
            run_main_file(&lua, main_file)?;
        }
        None => {
            if args.chunk.is_none() {
                repl(&lua);
            }
        }
    };

    if args.interactive && (!&args.main_file.is_none() || !&args.chunk.is_none()) {
        repl(&lua);
    }

    Ok(())
}

fn setup_komandan_table(lua: &Lua) -> mlua::Result<()> {
    let komandan = lua.create_table()?;

    komandan.set("defaults", defaults::defaults(&lua)?)?;

    let base_module = base_module(&lua);

    komandan.set("KomandanModule", base_module)?;

    komandan.set("set_defaults", lua.create_function(set_defaults)?)?;
    komandan.set("komando", lua.create_async_function(komando)?)?;

    // Add utils
    komandan.set("regex_is_match", lua.create_function(regex_is_match)?)?;
    komandan.set("filter_hosts", lua.create_function(filter_hosts)?)?;
    komandan.set("dprint", lua.create_function(dprint)?)?;

    // Add core modules
    let modules_table = lua.create_table()?;
    modules_table.set("apt", lua.create_function(apt)?)?;
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

    let host_display = hostname_display(&host);

    let task_display = match task.get::<String>("name") {
        Ok(name) => name,
        Err(_) => module.get::<String>("name")?,
    };

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

    let mut ssh = SSHSession::connect(
        (host.get::<String>("address")?.as_str(), port),
        &user,
        ssh_auth_method,
        elevation,
    )?;

    let env_defaults = defaults.get::<Table>("env").unwrap_or(lua.create_table()?);
    let env_host = host.get::<Table>("env").unwrap_or(lua.create_table()?);
    let env_task = task.get::<Table>("env").unwrap_or(lua.create_table()?);

    for pair in env_defaults.pairs() {
        let (key, value): (String, String) = pair?;
        println!("{}={}", key, value);
        ssh.set_env(&key, &value);
    }

    for pair in env_host.pairs() {
        let (key, value): (String, String) = pair?;
        println!("{}={}", key, value);
        ssh.set_env(&key, &value);
    }

    for pair in env_task.pairs() {
        let (key, value): (String, String) = pair?;
        println!("{}={}", key, value);
        ssh.set_env(&key, &value);
    }

    let module_clone = module.clone();
    let results = lua
        .load(chunk! {
            print("Running task '" .. $task_display .. "' on host '" .. $host_display .."' ...")
            $module_clone.ssh = $ssh
            $module_clone:run()

            local results = $module_clone.ssh:get_session_results()
            komandan.dprint(results.stdout)
            if results.exit_code ~= 0 then
                print("Task '" .. $task_display .. "' on host '" .. $host_display .."' failed with exit code " .. results.exit_code .. ": " .. results.stderr)
            else
                print("Task '" .. $task_display .. "' on host '" .. $host_display .."' succeeded.")
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

// Tests
#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(komandan_table.contains_key("dprint").unwrap());

        let modules_table = komandan_table.get::<Table>("modules").unwrap();
        assert!(modules_table.contains_key("apt").unwrap());
        assert!(modules_table.contains_key("cmd").unwrap());
        assert!(modules_table.contains_key("script").unwrap());
        assert!(modules_table.contains_key("upload").unwrap());
        assert!(modules_table.contains_key("download").unwrap());
    }

    #[test]
    fn test_set_defaults() {
        let lua = Lua::new();
        setup_komandan_table(&lua).unwrap();

        // Test setting a default value
        let defaults_data = lua.create_table().unwrap();
        defaults_data.set("user", "testuser").unwrap();
        set_defaults(&lua, Value::Table(defaults_data)).unwrap();

        let defaults = lua
            .globals()
            .get::<Table>("komandan")
            .unwrap()
            .get::<Table>("defaults")
            .unwrap();
        assert_eq!(defaults.get::<String>("user").unwrap(), "testuser");

        // Test setting multiple default values
        let defaults_data = lua.create_table().unwrap();
        defaults_data.set("port", 2222).unwrap();
        defaults_data.set("key", "/path/to/key").unwrap();
        set_defaults(&lua, Value::Table(defaults_data)).unwrap();

        let defaults = lua
            .globals()
            .get::<Table>("komandan")
            .unwrap()
            .get::<Table>("defaults")
            .unwrap();
        assert_eq!(defaults.get::<Integer>("port").unwrap(), 2222);
        assert_eq!(defaults.get::<String>("key").unwrap(), "/path/to/key");

        // Test with non-table input
        let result = set_defaults(
            &lua,
            Value::String(lua.create_string("not_a_table").unwrap()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_hostname_display() {
        let lua = Lua::new();

        // Test with name
        let host = lua.create_table().unwrap();
        host.set("address", "192.168.1.1").unwrap();
        host.set("name", "test").unwrap();
        assert_eq!(hostname_display(&host), "test (192.168.1.1)");

        // Test without name
        let host = lua.create_table().unwrap();
        host.set("address", "10.0.0.1").unwrap();
        assert_eq!(hostname_display(&host), "10.0.0.1");
    }

    #[test]
    fn test_run_main_file() {
        let lua = Lua::new();

        // Test with a valid Lua file
        let main_file = "examples/hosts.lua".to_string();
        let result = run_main_file(&lua, &main_file);
        assert!(result.is_ok());

        // Test with a non-Lua file
        let main_file = "README.md".to_string();
        let result = run_main_file(&lua, &main_file);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Main file must be a lua file."
        );
    }
}
