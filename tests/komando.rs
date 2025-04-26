use komandan::create_lua;
use mlua::{chunk, Integer, Table};
use std::{env, io::Write};
use tempfile::NamedTempFile;

#[test]
fn test_komando_invalid_known_hosts_path() {
    let lua = create_lua().unwrap();

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
                known_hosts_file = "/path/to/invalid/known_hosts"
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_err());
}

#[test]
fn test_komando_known_hosts_check_not_match() {
    let lua = create_lua().unwrap();

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost2",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_err());
}

#[test]
fn test_komando_userauth_invalid_password() {
    let lua = create_lua().unwrap();

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                password = "passw0rd"
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_err());
}

#[test]
fn test_komando_use_default_user() {
    let lua = create_lua().unwrap();

    let result = lua
        .load(chunk! {
            komandan.defaults:set_user("usertest")

            local hosts = {
                address = "localhost",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_ok());
}

#[test]
fn test_komando_use_default_user_from_env() {
    let lua = create_lua().unwrap();
    unsafe { env::set_var("USER", "usertest") };

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_ok());
}

#[test]
fn test_komando_no_user_specified() {
    let lua = create_lua().unwrap();
    unsafe { env::remove_var("USER") };

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>();

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No user specified for task"));
}

#[test]
fn test_komando_simple_cmd() {
    let lua = create_lua().unwrap();

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()
        .unwrap();

    assert!(result_table.get::<Integer>("exit_code").unwrap() == 0);
    assert!(result_table.get::<String>("stdout").unwrap() == "hello");
    assert!(result_table.get::<String>("stderr").unwrap() == "");
}

#[test]
fn test_komando_simple_script() {
    let lua = create_lua().unwrap();

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.script({
                    script = "echo hello",
                    interpreter = "sh"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()
        .unwrap();

    assert!(result_table.get::<Integer>("exit_code").unwrap() == 0);
    assert!(result_table.get::<String>("stdout").unwrap() == "hello");
    assert!(result_table.get::<String>("stderr").unwrap() == "");
}

#[test]
fn test_komando_script_from_file() {
    let lua = create_lua().unwrap();

    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "echo hello").unwrap();

    let temp_file_path = temp_file.path().to_str().unwrap();

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.script({
                    from_file = $temp_file_path,
                    interpreter = "sh"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()
        .unwrap();

    assert!(result_table.get::<Integer>("exit_code").unwrap() == 0);
    assert!(result_table.get::<String>("stdout").unwrap() == "hello");
    assert!(result_table.get::<String>("stderr").unwrap() == "");
}

#[test]
fn test_komando_apt() {
    let lua = create_lua().unwrap();

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.apt({
                    package = "tar",
                }),
                elevate = true
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()
        .unwrap();

    assert!(result_table.get::<Integer>("exit_code").unwrap() == 0);
}
