mod args;
mod defaults;
mod modules;
pub mod ssh;
mod util;
mod validator;

use args::Args;
use clap::Parser;
use defaults::Defaults;
use mlua::{
    chunk,
    Error::{self, RuntimeError},
    FromLua, Integer, IntoLua, Lua, LuaSerdeExt, MultiValue, Table, UserData, Value,
};
use modules::{base_module, collect_modules};
use rayon::prelude::*;
use rustyline::DefaultEditor;
use serde::{Deserialize, Serialize};
use ssh::{Elevation, ElevationMethod, SSHAuthMethod, SSHSession};
use std::{collections::HashMap, env, fs, path::Path};
use util::{
    dprint, filter_hosts, host_display, parse_hosts_json_file, parse_hosts_json_url,
    regex_is_match, task_display,
};
use validator::{validate_host, validate_task};

pub fn create_lua() -> mlua::Result<Lua> {
    let lua = Lua::new();
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

    Ok(lua)
}

pub fn setup_komandan_table(lua: &Lua) -> mlua::Result<()> {
    let komandan = lua.create_table()?;

    let defaults = Defaults::global();
    komandan.set("defaults", defaults)?;

    let base_module = base_module(&lua);
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

    // Add core modules
    komandan.set("modules", collect_modules(lua))?;

    lua.globals().set("komandan", &komandan)?;

    Ok(())
}

fn get_user(host: &Table, task: &Table) -> mlua::Result<String> {
    let defaults = Defaults::global();
    let default_user = match defaults.user.read() {
        Ok(user) => user,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };
    let user = match host.get::<String>("user") {
        Ok(user) => user,
        Err(_) => match *default_user {
            Some(ref user) => user.clone(),
            None => match env::var("USER") {
                Ok(user) => user,
                Err(_) => {
                    return Err(RuntimeError(format!(
                        "No user specified for task '{}'.",
                        task_display(task)
                    )))
                }
            },
        },
    };

    Ok(user)
}

fn get_auth_config(host: &Table, task: &Table) -> mlua::Result<(String, SSHAuthMethod)> {
    let host_display = host_display(host);
    let task_display = task_display(task);

    let user = get_user(host, task)?;

    let defaults = Defaults::global();

    let default_private_key_file = match defaults.private_key_file.read() {
        Ok(private_key_file) => private_key_file,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let default_private_key_pass = match defaults.private_key_pass.read() {
        Ok(private_key_pass) => private_key_pass,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let default_password = match defaults.password.read() {
        Ok(password) => password,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let ssh_auth_method = match host.get::<String>("private_key_file") {
        Ok(private_key_file) => SSHAuthMethod::PublicKey {
            private_key: private_key_file,
            passphrase: match host.get::<String>("private_key_pass") {
                Ok(passphrase) => Some(passphrase),
                Err(_) => match *default_private_key_pass {
                    Some(ref private_key_pass) => Some(private_key_pass.clone()),
                    None => None,
                },
            },
        },
        Err(_) => match *default_private_key_file {
            Some(ref private_key_file) => SSHAuthMethod::PublicKey {
                private_key: private_key_file.clone(),
                passphrase: match host.get::<String>("private_key_pass") {
                    Ok(passphrase) => Some(passphrase),
                    Err(_) => match *default_private_key_pass {
                        Some(ref passphrase) => Some(passphrase.clone()),
                        None => None,
                    },
                },
            },
            None => match host.get::<String>("password") {
                Ok(password) => SSHAuthMethod::Password(password),
                Err(_) => match *default_password {
                    Some(ref password) => SSHAuthMethod::Password(password.clone()),
                    None => {
                        return Err(RuntimeError(format!(
                            "No authentication method specified for task '{}' on host '{}'.",
                            task_display, host_display
                        )))
                    }
                },
            },
        },
    };

    Ok((user, ssh_auth_method))
}

fn get_elevation_config(host: &Table, task: &Table) -> mlua::Result<Elevation> {
    let defaults = Defaults::global();

    let default_elevate = match defaults.elevate.read() {
        Ok(elevate) => elevate,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let task_elevate = task.get::<Value>("elevate")?;
    let host_elevate = host.get::<Value>("elevate")?;

    let elevate = if !task_elevate.is_nil() {
        task_elevate.as_boolean().unwrap()
    } else if !host_elevate.is_nil() {
        host_elevate.as_boolean().unwrap()
    } else {
        *default_elevate
    };

    if !elevate {
        return Ok(Elevation {
            method: ElevationMethod::None,
            as_user: None,
        });
    }

    let default_elevation_method = match defaults.elevation_method.read() {
        Ok(elevation_method) => elevation_method,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let elevation_method_str = task.get::<String>("elevation_method").unwrap_or(
        host.get::<String>("elevation_method")
            .unwrap_or(default_elevation_method.clone()),
    );

    let elevation_method = match elevation_method_str.as_str() {
        "none" => Ok(ElevationMethod::None),
        "sudo" => Ok(ElevationMethod::Sudo),
        "su" => Ok(ElevationMethod::Su),
        _ => Err(RuntimeError(format!(
            "Unsupported elevation method: {}",
            elevation_method_str
        ))),
    };

    let default_as_user = match defaults.as_user.read() {
        Ok(as_user) => as_user,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let as_user = task.get::<Option<String>>("as_user").unwrap_or(
        host.get::<Option<String>>("as_user")
            .unwrap_or(default_as_user.clone()),
    );

    Ok(Elevation {
        method: elevation_method?,
        as_user,
    })
}

fn setup_ssh_session(host: &Table) -> mlua::Result<SSHSession> {
    let defaults = Defaults::global();
    let mut ssh = SSHSession::new()?;

    let default_host_key_check = match defaults.host_key_check.read() {
        Ok(host_key_check) => host_key_check,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let host_key_check = match host.get::<Value>("host_key_check") {
        Ok(host_key_check) => match host_key_check {
            Value::Nil => default_host_key_check.clone(),
            Value::Boolean(false) => false,
            _ => true,
        },
        Err(_) => true,
    };

    let default_known_hosts_file = match defaults.known_hosts_file.read() {
        Ok(known_hosts_file) => known_hosts_file,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    if host_key_check {
        ssh.known_hosts_file = match host.get::<String>("known_hosts_file") {
            Ok(known_hosts_file) => Some(known_hosts_file),
            Err(_) => Some(default_known_hosts_file.clone()),
        };
    }

    Ok(ssh)
}

fn setup_environment(ssh: &mut SSHSession, host: &Table, task: &Table) -> mlua::Result<()> {
    let defaults = Defaults::global();

    let default_env = match defaults.env.read() {
        Ok(env) => env,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let env_host = host.get::<Option<Table>>("env")?;
    let env_task = task.get::<Option<Table>>("env")?;

    for (key, value) in default_env.clone() {
        ssh.set_env(&key, &value);
    }

    if !env_host.is_none() {
        for pair in env_host.unwrap().pairs() {
            let (key, value): (String, String) = pair?;
            ssh.set_env(&key, &value);
        }
    }

    if !env_task.is_none() {
        for pair in env_task.unwrap().pairs() {
            let (key, value): (String, String) = pair?;
            ssh.set_env(&key, &value);
        }
    }

    Ok(())
}

fn execute_task(
    lua: &Lua,
    module: &Table,
    ssh: SSHSession,
    task_display: &str,
    host_display: &str,
) -> mlua::Result<Table> {
    lua.load(chunk! {
        print(">> Running task '" .. $task_display .. "' on host '" .. $host_display .."' ...")
        $module.ssh = $ssh
        $module:run()

        local results = $module.ssh:get_session_results()
        komandan.dprint(results.stdout)
        if results.exit_code ~= 0 then
            print(">> Task '" .. $task_display .. "' on host '" .. $host_display .."' failed with exit code " .. results.exit_code .. ": " .. results.stderr)
        else
            print(">> Task '" .. $task_display .. "' on host '" .. $host_display .."' succeeded.")
        end

        if $module.cleanup ~= nil then
            $module:cleanup()
        end

        return results
    })
    .set_name("execute_task")
    .eval::<Table>()
}

fn komando(lua: &Lua, (host, task): (Value, Value)) -> mlua::Result<Table> {
    let host = lua.create_function(validate_host)?.call::<Table>(&host)?;
    let task = lua.create_function(validate_task)?.call::<Table>(&task)?;
    let module = task.get::<Table>(1)?;

    let host_display = host_display(&host);
    let task_display = task_display(&task);

    let defaults = Defaults::global();

    let (user, ssh_auth_method) = get_auth_config(&host, &task)?;
    let elevation = get_elevation_config(&host, &task)?;

    let default_port = match defaults.port.read() {
        Ok(port) => port,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let port = host
        .get::<Integer>("port")
        .unwrap_or(default_port.clone() as i64) as u16;

    let mut ssh = setup_ssh_session(&host)?;
    ssh.elevation = elevation;

    ssh.connect(
        host.get::<String>("address")?.as_str(),
        port,
        &user,
        ssh_auth_method,
    )?;

    setup_environment(&mut ssh, &host, &task)?;

    let results = execute_task(lua, &module, ssh, &task_display, &host_display)?;

    let default_ignore_exit_code = match defaults.ignore_exit_code.read() {
        Ok(ignore_exit_code) => ignore_exit_code,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let ignore_exit_code = task
        .get::<bool>("ignore_exit_code")
        .unwrap_or(default_ignore_exit_code.clone());

    if results.get::<Integer>("exit_code").unwrap() != 0 && !ignore_exit_code {
        return Err(RuntimeError("Failed to run task.".to_string()));
    }

    Ok(results)
}

#[derive(Clone, Debug)]
struct Host {
    name: Option<String>,
    address: String,
    port: Option<u16>,
    user: Option<String>,
    private_key_file: Option<String>,
    private_key_pass: Option<String>,
    password: Option<String>,
    elevate: Option<bool>,
    elevation_method: Option<ElevationMethod>,
    as_user: Option<String>,
    env: Option<HashMap<String, String>>,
}

impl FromLua for Host {
    fn from_lua(lua_value: Value, _: &Lua) -> mlua::Result<Self> {
        let table = lua_value.as_table().unwrap();
        Ok(Host {
            name: table.get("name")?,
            address: table.get("address")?,
            port: table.get("port")?,
            user: table.get("user")?,
            private_key_file: table.get("private_key_file")?,
            private_key_pass: table.get("private_key_pass")?,
            password: table.get("password")?,
            elevate: table.get("elevate")?,
            elevation_method: match table.get::<String>("elevation_method") {
                Ok(elevation_method) => match elevation_method.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                },
                Err(_) => None,
            },
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Host {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if self.name.is_some() {
            table.set("name", self.name.unwrap())?;
        }
        table.set("address", self.address)?;
        if self.port.is_some() {
            table.set("port", self.port.unwrap())?;
        }
        if self.user.is_some() {
            table.set("user", self.user.unwrap())?;
        }
        if self.private_key_file.is_some() {
            table.set("private_key_file", self.private_key_file.unwrap())?;
        }
        if self.private_key_pass.is_some() {
            table.set("private_key_pass", self.private_key_pass.unwrap())?;
        }
        if self.password.is_some() {
            table.set("password", self.password.unwrap())?;
        }
        if self.elevate.is_some() {
            table.set("elevate", self.elevate.unwrap())?;
        }
        if self.elevation_method.is_some() {
            match self.elevation_method.unwrap() {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if self.as_user.is_some() {
            table.set("as_user", self.as_user.unwrap())?;
        }
        if self.env.is_some() {
            table.set("env", self.env.unwrap())?;
        }
        Ok(Value::Table(table))
    }
}

#[derive(Clone, Debug)]
struct Task {
    name: Option<String>,
    module: Module,
    ignore_exit_code: Option<bool>,
    elevate: Option<bool>,
    elevation_method: Option<ElevationMethod>,
    as_user: Option<String>,
    env: Option<HashMap<String, String>>,
}

impl FromLua for Task {
    fn from_lua(lua_value: Value, lua: &Lua) -> mlua::Result<Self> {
        let table = lua_value.as_table().unwrap();
        Ok(Task {
            name: table.get("name")?,
            module: Module::from_lua(table.get(1)?, lua)?,
            ignore_exit_code: table.get("ignore_exit_code")?,
            elevate: table.get("elevate")?,
            elevation_method: match table.get::<String>("elevation_method") {
                Ok(elevation_method) => match elevation_method.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                },
                Err(_) => None,
            },
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Task {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if self.name.is_some() {
            table.set("name", self.name.unwrap())?;
        }
        table.set(1, self.module.into_lua(lua)?)?;
        if self.ignore_exit_code.is_some() {
            table.set("ignore_exit_code", self.ignore_exit_code.unwrap())?;
        }
        if self.elevate.is_some() {
            table.set("elevate", self.elevate.unwrap())?;
        }
        if self.elevation_method.is_some() {
            match self.elevation_method.unwrap() {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if self.as_user.is_some() {
            table.set("as_user", self.as_user.unwrap())?;
        }
        if self.env.is_some() {
            table.set("env", self.env.unwrap())?;
        }
        Ok(Value::Table(table))
    }
}

#[derive(Clone, Debug)]
struct Module {
    functions: HashMap<String, Vec<u8>>,
    others: HashMap<String, String>,
}

impl FromLua for Module {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        let table = value.as_table().unwrap();
        let mut functions: HashMap<String, Vec<u8>> = HashMap::new();
        let mut others: HashMap<String, String> = HashMap::new();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair.unwrap();
            if value.is_function() {
                functions.insert(key.to_string()?, value.as_function().unwrap().dump(true));
            } else {
                others.insert(
                    key.to_string()?,
                    serde_json::to_string(&value).map_err(Error::external)?,
                );
            }
        }
        Ok(Module { functions, others })
    }
}

impl IntoLua for Module {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        self.functions.iter().for_each(|(key, value)| {
            table
                .set(key.as_str(), lua.load(value).into_function().unwrap())
                .unwrap();
        });
        self.others.iter().for_each(|(key, value)| {
            let json: serde_json::Value = serde_json::from_str(value).unwrap();
            table
                .set(key.as_str(), lua.to_value(&json).unwrap())
                .unwrap();
        });
        Ok(Value::Table(table))
    }
}

#[derive(Serialize, Deserialize)]
struct KomandoResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

impl UserData for KomandoResult {}

fn komando_parallel_tasks(lua: &Lua, (host, tasks): (Value, Value)) -> mlua::Result<Table> {
    let host = Host::from_lua(host, lua)?;
    let mut tasks_hm = HashMap::<u32, Task>::new();
    for pair in tasks.as_table().unwrap().pairs::<u32, Value>() {
        let (key, value): (u32, Value) = pair?;
        let task = Task::from_lua(value, lua)?;
        tasks_hm.insert(key, task);
    }

    let results: HashMap<u32, KomandoResult> = tasks_hm
        .par_iter()
        .map(|(i, task)| {
            let lua = create_lua().unwrap();
            let host = host.clone().into_lua(&lua).unwrap();
            let task = task.clone().into_lua(&lua).unwrap();
            let result = komando(&lua, (host, task)).unwrap();

            return (
                *i,
                lua.from_value::<KomandoResult>(Value::Table(result))
                    .unwrap(),
            );
        })
        .collect::<HashMap<u32, KomandoResult>>();

    let results_table = lua.create_table()?;
    results.iter().for_each(|(i, result)| {
        results_table
            .set(*i, lua.to_value(result).unwrap())
            .unwrap();
    });

    Ok(results_table)
}

fn komando_parallel_hosts(lua: &Lua, (hosts, task): (Value, Value)) -> mlua::Result<Table> {
    let task = Task::from_lua(task, lua)?;
    let mut hosts_hm = HashMap::<u32, Host>::new();
    for pair in hosts.as_table().unwrap().pairs::<u32, Value>() {
        let (key, value): (u32, Value) = pair?;
        let host = Host::from_lua(value, lua)?;
        hosts_hm.insert(key, host);
    }

    let results: HashMap<u32, KomandoResult> = hosts_hm
        .par_iter()
        .map(|(i, host)| {
            let lua = create_lua().unwrap();
            let host = host.clone().into_lua(&lua).unwrap();
            let task = task.clone().into_lua(&lua).unwrap();
            let result = komando(&lua, (host, task)).unwrap();

            return (
                *i,
                lua.from_value::<KomandoResult>(Value::Table(result))
                    .unwrap(),
            );
        })
        .collect::<HashMap<u32, KomandoResult>>();

    let results_table = lua.create_table()?;
    results.iter().for_each(|(i, result)| {
        results_table
            .set(*i, lua.to_value(result).unwrap())
            .unwrap();
    });

    Ok(results_table)
}

pub fn run_main_file(lua: &Lua, main_file: &String) -> anyhow::Result<()> {
    let script = match fs::read_to_string(main_file) {
        Ok(script) => script,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to read the main file ({}): {}",
                main_file,
                e
            ));
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
        let lua = create_lua().unwrap();

        // Assert that the komandan table is set up correctly
        let komandan_table = lua.globals().get::<Table>("komandan").unwrap();
        assert!(komandan_table.contains_key("defaults").unwrap());
        assert!(komandan_table.contains_key("KomandanModule").unwrap());
        assert!(komandan_table.contains_key("komando").unwrap());
        assert!(komandan_table.contains_key("regex_is_match").unwrap());
        assert!(komandan_table.contains_key("filter_hosts").unwrap());
        assert!(komandan_table
            .contains_key("parse_hosts_json_file")
            .unwrap());
        assert!(komandan_table.contains_key("parse_hosts_json_url").unwrap());
        assert!(komandan_table.contains_key("dprint").unwrap());

        let modules_table = komandan_table.get::<Table>("modules").unwrap();
        assert!(modules_table.contains_key("apt").unwrap());
        assert!(modules_table.contains_key("cmd").unwrap());
        assert!(modules_table.contains_key("lineinfile").unwrap());
        assert!(modules_table.contains_key("script").unwrap());
        assert!(modules_table.contains_key("systemd_service").unwrap());
        assert!(modules_table.contains_key("template").unwrap());
        assert!(modules_table.contains_key("upload").unwrap());
        assert!(modules_table.contains_key("download").unwrap());
    }

    #[test]
    fn test_run_main_file() {
        let lua = create_lua().unwrap();

        // Test with a valid Lua file
        let main_file = "examples/hosts.lua".to_string();
        let result = run_main_file(&lua, &main_file);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_auth_config() {
        let lua = create_lua().unwrap();
        let host = lua.create_table().unwrap();

        // Test with user in host
        host.set("address", "localhost").unwrap();
        host.set("user", "testuser").unwrap();
        host.set("private_key_file", "/path/to/key").unwrap();

        let module_params = lua.create_table().unwrap();
        module_params.set("cmd", "echo test").unwrap();
        let module = lua
            .load(chunk! {
                return komandan.modules.cmd($module_params)
            })
            .eval::<Table>()
            .unwrap();
        let task = lua.create_table().unwrap();
        task.set(1, module).unwrap();

        let (user, auth) = get_auth_config(&host, &task).unwrap();
        assert_eq!(user, "testuser");
        match auth {
            SSHAuthMethod::PublicKey {
                private_key,
                passphrase,
            } => {
                assert_eq!(private_key, "/path/to/key");
                assert!(passphrase.is_none());
            }
            _ => panic!("Expected PublicKey authentication"),
        }

        // Test with password auth
        host.set("private_key_file", Value::Nil).unwrap();
        host.set("password", "testpass").unwrap();
        let (_, auth) = get_auth_config(&host, &task).unwrap();
        match auth {
            SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
            _ => panic!("Expected Password authentication"),
        }

        // Test with no authentication method
        host.set("password", Value::Nil).unwrap();
        let result = get_auth_config(&host, &task);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_elevation_config() {
        let lua = create_lua().unwrap();
        let host = lua.create_table().unwrap();
        let task = lua.create_table().unwrap();

        // Test with no elevation
        let elevation = get_elevation_config(&host, &task).unwrap();
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::None,
                as_user: None
            }
        ));

        // Test with elevation from task
        task.set("elevate", true).unwrap();
        let elevation = get_elevation_config(&host, &task).unwrap();
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::Sudo,
                as_user: None
            }
        ));

        // Test with custom elevation method
        task.set("elevation_method", "su").unwrap();
        let elevation = get_elevation_config(&host, &task).unwrap();
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::Su,
                as_user: None
            }
        ));

        // Test invalid elevation method
        task.set("elevation_method", "invalid").unwrap();
        assert!(get_elevation_config(&host, &task).is_err());
    }

    #[test]
    fn test_setup_ssh_session() {
        let lua = create_lua().unwrap();
        let host = lua.create_table().unwrap();
        host.set("address", "localhost").unwrap();

        // Test with default settings
        let ssh = setup_ssh_session(&host).unwrap();
        assert!(ssh.known_hosts_file.is_some());

        // Test with host key check disabled
        host.set("host_key_check", false).unwrap();
        let ssh = setup_ssh_session(&host).unwrap();
        assert!(ssh.known_hosts_file.is_none());

        // Test with custom known_hosts file
        host.set("known_hosts_file", "/path/to/known_hosts")
            .unwrap();
        host.set("host_key_check", true).unwrap();
        let ssh = setup_ssh_session(&host).unwrap();
        assert_eq!(ssh.known_hosts_file.unwrap(), "/path/to/known_hosts");

        // Test with known_hosts from defaults
        host.set("known_hosts_file", Value::Nil).unwrap();
        lua.load(chunk! {
            komandan.defaults:set_known_hosts_file("/default/known_hosts")
        })
        .exec()
        .unwrap();
        let ssh = setup_ssh_session(&host).unwrap();
        assert_eq!(ssh.known_hosts_file.unwrap(), "/default/known_hosts");
    }

    #[test]
    fn test_setup_environment() {
        let lua = create_lua().unwrap();
        let mut ssh = SSHSession::new().unwrap();
        let defaults = lua.create_table().unwrap();
        let host = lua.create_table().unwrap();
        let task = lua.create_table().unwrap();

        // Test with environment variables at all levels
        let env_defaults = lua.create_table().unwrap();
        env_defaults.set("DEFAULT_VAR", "default_value").unwrap();
        defaults.set("env", env_defaults).unwrap();

        let env_host = lua.create_table().unwrap();
        env_host.set("HOST_VAR", "host_value").unwrap();
        env_host.set("DEFAULT_VAR", "overridden_value").unwrap(); // Override default
        host.set("env", env_host).unwrap();

        let env_task = lua.create_table().unwrap();
        env_task.set("TASK_VAR", "task_value").unwrap();
        task.set("env", env_task).unwrap();

        setup_environment(&mut ssh, &host, &task).unwrap();
    }
}
