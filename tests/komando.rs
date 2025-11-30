use komandan::create_lua;
use mlua::{Integer, Table, chunk};
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_komando_invalid_known_hosts_path() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
                known_hosts_file = "/path/to/invalid/known_hosts",
                connection = "ssh"
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
    Ok(())
}

#[test]
fn test_komando_known_hosts_check_not_match() -> mlua::Result<()> {
    let lua = create_lua()?;

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
    Ok(())
}

#[test]
fn test_komando_userauth_invalid_password() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                password = "passw0rd",
                connection = "ssh"
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
    Ok(())
}

#[test]
fn test_komando_use_default_user() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result = lua
        .load(chunk! {
            komandan.defaults:set_user("usertest")

            local hosts = {
                address = "localhost",
                host_key_check = false,
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
    Ok(())
}

#[test]
#[allow(unsafe_code)]
fn test_komando_use_default_user_from_env() -> mlua::Result<()> {
    let lua = create_lua()?;
    unsafe { std::env::set_var("USER", "usertest") };

    let result = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                host_key_check = false,
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
    Ok(())
}

#[test]
fn test_komando_simple_cmd() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                host_key_check = false,
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.cmd({
                    cmd = "echo hello"
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()?;

    assert!(result_table.get::<Integer>("exit_code")? == 0);
    assert!(result_table.get::<String>("stdout")? == "hello");
    assert!((result_table.get::<String>("stderr")?).is_empty());
    Ok(())
}

#[test]
fn test_komando_simple_script() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                host_key_check = false,
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
        .eval::<Table>()?;

    assert!(result_table.get::<Integer>("exit_code")? == 0);
    assert!(result_table.get::<String>("stdout")? == "hello");
    assert!((result_table.get::<String>("stderr")?).is_empty());
    Ok(())
}

#[test]
fn test_komando_script_from_file() -> mlua::Result<()> {
    let lua = create_lua()?;

    let mut temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
    writeln!(temp_file, "echo hello").map_err(mlua::Error::external)?;

    let temp_file_path = temp_file
        .path()
        .to_str()
        .ok_or_else(|| mlua::Error::external("invalid path"))?;

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                host_key_check = false,
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
        .eval::<Table>()?;

    assert!(result_table.get::<Integer>("exit_code")? == 0);
    assert!(result_table.get::<String>("stdout")? == "hello");
    assert!((result_table.get::<String>("stderr")?).is_empty());
    Ok(())
}

#[test]
fn test_komando_apt() -> mlua::Result<()> {
    // Skip test if apt is not available (e.g., on non-Debian systems)
    if std::process::Command::new("which")
        .arg("apt")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("Skipping test_komando_apt: apt not available on this system");
        return Ok(());
    }

    let lua = create_lua()?;

    let result_table = lua
        .load(chunk! {
            local hosts = {
                address = "localhost",
                user = "usertest",
                host_key_check = false,
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519"
            }

            local task = {
                komandan.modules.apt({
                    package = "tar",
                })
            }

            return komandan.komando(hosts, task)
        })
        .eval::<Table>()?;

    assert!(result_table.get::<Integer>("exit_code")? == 0);
    Ok(())
}
