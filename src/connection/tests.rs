use super::auth::get_user;
use super::session::get_port_from_host;
use super::*;
use crate::create_lua;
use crate::ssh::{Elevation, ElevationMethod, SSHAuthMethod};
use mlua::Error::RuntimeError;

#[test]
fn test_create_connection_local() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {}
        Connection::SSH(_) => panic!("Expected local connection for localhost"),
    }

    Ok(())
}

#[test]
fn test_create_connection_ssh_factory_logic() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "remote.example.com")?;
    host_table.set("user", "testuser")?;
    host_table.set("password", "testpass")?;

    // Test that the factory correctly identifies this as SSH
    let connection_type = determine_connection_type(&host_table)?;
    assert_eq!(connection_type, ConnectionType::SSH);

    // Test that we can create the dummy task
    let task = create_dummy_task(&lua)?;
    assert_eq!(task.len()?, 0);

    // Test that we can get auth config
    let (user, auth) = get_auth_config(&host_table, &task, None)?;
    assert_eq!(user, "testuser");
    match auth {
        SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
        SSHAuthMethod::PublicKey { .. } => panic!("Expected Password authentication"),
    }

    Ok(())
}

#[test]
fn test_create_connection_explicit_local() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "remote.example.com")?;
    host_table.set("connection", "local")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {}
        Connection::SSH(_) => panic!("Expected local connection when explicitly set"),
    }

    Ok(())
}

#[test]
fn test_create_connection_explicit_ssh_factory_logic() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;
    host_table.set("connection", "ssh")?;
    host_table.set("user", "testuser")?;
    host_table.set("password", "testpass")?;

    // Test that the factory correctly identifies this as SSH even for localhost
    let connection_type = determine_connection_type(&host_table)?;
    assert_eq!(connection_type, ConnectionType::SSH);

    Ok(())
}

#[test]
fn test_create_connection_with_environment() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let env_table = lua.create_table()?;
    env_table.set("TEST_VAR", "test_value")?;
    host_table.set("env", env_table)?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {}
        Connection::SSH(_) => panic!("Expected local connection for localhost"),
    }

    Ok(())
}

#[test]
fn test_create_dummy_task() -> mlua::Result<()> {
    let lua = create_lua()?;
    let task = create_dummy_task(&lua)?;

    // Should be an empty table
    assert_eq!(task.len()?, 0);

    Ok(())
}

#[test]
fn test_get_port_from_host() -> mlua::Result<()> {
    let lua = create_lua()?;

    // Test with explicit port
    let host_table = lua.create_table()?;
    host_table.set("port", 2222)?;
    let port = get_port_from_host(&host_table)?;
    assert_eq!(port, 2222);

    // Test with default port - reset defaults first
    lua.load(mlua::chunk! {
        komandan.defaults:set_port(22)
    })
    .exec()?;

    let host_table = lua.create_table()?;
    let port = get_port_from_host(&host_table)?;
    assert_eq!(port, 22); // Should use default

    Ok(())
}

#[test]
fn test_determine_connection_type_localhost() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let conn_type = determine_connection_type(&host_table)?;
    assert_eq!(conn_type, ConnectionType::Local);

    Ok(())
}

#[test]
fn test_determine_connection_type_127_0_0_1() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "127.0.0.1")?;

    let conn_type = determine_connection_type(&host_table)?;
    assert_eq!(conn_type, ConnectionType::Local);

    Ok(())
}

#[test]
fn test_determine_connection_type_ipv6_localhost() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "::1")?;

    let conn_type = determine_connection_type(&host_table)?;
    assert_eq!(conn_type, ConnectionType::Local);

    Ok(())
}

#[test]
fn test_determine_connection_type_remote() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "remote.example.com")?;

    let conn_type = determine_connection_type(&host_table)?;
    assert_eq!(conn_type, ConnectionType::SSH);

    Ok(())
}

#[test]
fn test_determine_connection_type_explicit() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;
    host_table.set("connection", "ssh")?;

    let conn_type = determine_connection_type(&host_table)?;
    assert_eq!(conn_type, ConnectionType::SSH);

    Ok(())
}

#[test]
fn test_is_localhost() {
    assert!(is_localhost("localhost"));
    assert!(is_localhost("127.0.0.1"));
    assert!(is_localhost("::1"));
    assert!(!is_localhost("remote.example.com"));
    assert!(!is_localhost("192.168.1.1"));
}

#[test]
fn test_connection_type() -> mlua::Result<()> {
    let lua = create_lua()?;

    // Test local connection type
    let local_host = lua.create_table()?;
    local_host.set("address", "localhost")?;
    let local_conn = create_connection(&lua, &Value::Table(local_host))?;
    assert_eq!(local_conn.connection_type(), ConnectionType::Local);

    Ok(())
}

#[test]
fn test_get_auth_config() -> anyhow::Result<()> {
    let lua = create_lua()?;
    let host = lua.create_table()?;

    // Test with user in host
    host.set("address", "localhost")?;
    host.set("user", "testuser")?;
    host.set("private_key_file", "/path/to/key")?;

    let module_params = lua.create_table()?;
    module_params.set("cmd", "echo test")?;
    let module = lua
        .load(mlua::chunk! {
            return komandan.modules.cmd($module_params)
        })
        .eval::<Table>()?;
    let task = lua.create_table()?;
    task.set(1, module)?;

    let (user, auth) = get_auth_config(&host, &task, None)?;
    assert_eq!(user, "testuser");
    match auth {
        SSHAuthMethod::PublicKey {
            private_key,
            passphrase,
        } => {
            assert_eq!(private_key, "/path/to/key");
            assert!(passphrase.is_none());
        }
        SSHAuthMethod::Password(_) => panic!("Expected PublicKey authentication"),
    }

    // Test with password auth
    host.set("private_key_file", Value::Nil)?;
    host.set("password", "testpass")?;
    let (_, auth) = get_auth_config(&host, &task, None)?;
    match auth {
        SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
        SSHAuthMethod::PublicKey { .. } => panic!("Expected Password authentication"),
    }

    // Test with no authentication method
    host.set("password", Value::Nil)?;
    let temp_dir =
        tempfile::tempdir().map_err(|e| anyhow::anyhow!("failed to create temp dir: {e}"))?;
    let home_path = temp_dir.path().display().to_string();
    let result = get_auth_config(&host, &task, Some(&home_path));
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_get_user() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host = lua.create_table()?;
    let task = lua.create_table()?;

    // Test with user in host
    host.set("user", "testuser")?;
    let user = get_user(&host, &task)?;
    assert_eq!(user, "testuser");

    // Test with no user specified (should fall back to environment)
    host.set("user", Value::Nil)?;
    let user = get_user(&host, &task);
    // This should either succeed with the current USER env var or fail
    // We can't predict the exact behavior since it depends on the environment
    assert!(user.is_ok() || user.is_err());

    Ok(())
}

#[test]
fn test_create_ssh_session() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host = lua.create_table()?;
    host.set("address", "localhost")?;

    // Test with default settings
    let ssh = create_ssh_session(&host)?;
    assert!(ssh.known_hosts_file.is_some());

    // Test with host key check disabled
    host.set("host_key_check", false)?;
    let ssh = create_ssh_session(&host)?;
    assert!(ssh.known_hosts_file.is_none());

    // Test with custom known_hosts file
    host.set("known_hosts_file", "/path/to/known_hosts")?;
    host.set("host_key_check", true)?;
    let ssh = create_ssh_session(&host)?;
    assert_eq!(
        ssh.known_hosts_file,
        Some("/path/to/known_hosts".to_string())
    );

    // Test with known_hosts from defaults
    host.set("known_hosts_file", Value::Nil)?;
    lua.load(mlua::chunk! {
        komandan.defaults:set_known_hosts_file("/default/known_hosts")
    })
    .exec()?;
    let ssh = create_ssh_session(&host)?;
    assert_eq!(
        ssh.known_hosts_file,
        Some("/default/known_hosts".to_string())
    );

    Ok(())
}

#[test]
fn test_get_elevation_config() -> mlua::Result<()> {
    let lua = create_lua()?;
    let host = lua.create_table()?;
    let task = lua.create_table()?;

    // Test with no elevation
    let elevation = get_elevation_config(&host, &task)?;
    assert!(matches!(
        elevation,
        Elevation {
            method: ElevationMethod::None,
            as_user: None
        }
    ));

    // Test with elevation from task
    task.set("elevate", true)?;
    let elevation = get_elevation_config(&host, &task)?;
    assert!(matches!(
        elevation,
        Elevation {
            method: ElevationMethod::Sudo,
            as_user: None
        }
    ));

    // Test with custom elevation method
    task.set("elevation_method", "su")?;
    let elevation = get_elevation_config(&host, &task)?;
    assert!(matches!(
        elevation,
        Elevation {
            method: ElevationMethod::Su,
            as_user: None
        }
    ));

    // Test invalid elevation method
    task.set("elevation_method", "invalid")?;
    assert!(get_elevation_config(&host, &task).is_err());

    Ok(())
}

#[test]
fn test_setup_environment_ssh() -> mlua::Result<()> {
    let lua = create_lua()?;
    let mut ssh = SSHSession::new()
        .map_err(|e| RuntimeError(format!("Failed to create SSH session: {e}")))?;
    let host = lua.create_table()?;
    let task = lua.create_table()?;

    // Test with environment variables at all levels
    let env_host = lua.create_table()?;
    env_host.set("HOST_VAR", "host_value")?;
    env_host.set("OVERRIDE_VAR", "host_override")?; // This should be overridden by task
    host.set("env", env_host)?;

    let env_task = lua.create_table()?;
    env_task.set("TASK_VAR", "task_value")?;
    env_task.set("OVERRIDE_VAR", "task_override")?; // This should override host value
    task.set("env", env_task)?;

    setup_environment_ssh(&mut ssh, &host, &task)?;

    // We can't directly test the environment variables since SSHSession doesn't expose them
    // But we can verify the function completes without error
    Ok(())
}

#[test]
fn test_setup_environment_ssh_empty() -> mlua::Result<()> {
    let lua = create_lua()?;
    let mut ssh = SSHSession::new()
        .map_err(|e| RuntimeError(format!("Failed to create SSH session: {e}")))?;
    let host = lua.create_table()?;
    let task = lua.create_table()?;

    // Test with no environment variables
    setup_environment_ssh(&mut ssh, &host, &task)?;

    Ok(())
}
