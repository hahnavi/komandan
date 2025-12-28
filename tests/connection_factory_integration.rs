use anyhow::Result;
use komandan::connection::{Connection, create_connection};
use komandan::create_lua;
use komandan::executor::CommandExecutor;
use mlua::Value;
use std::env;

/// Integration tests for the connection factory
/// These tests verify that the connection factory works correctly with real configurations
/// and integrates properly with the existing SSH and local session infrastructure.

#[test]
fn test_connection_factory_local_connection() -> Result<()> {
    let lua = create_lua()?;

    // Test localhost address defaults to local connection
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {
            // Success - localhost should create local connection
        }
        Connection::SSH(_) => {
            panic!("Expected local connection for localhost address");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_127_0_0_1() -> Result<()> {
    let lua = create_lua()?;

    // Test 127.0.0.1 address defaults to local connection
    let host_table = lua.create_table()?;
    host_table.set("address", "127.0.0.1")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {
            // Success - 127.0.0.1 should create local connection
        }
        Connection::SSH(_) => {
            panic!("Expected local connection for 127.0.0.1 address");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_ipv6_localhost() -> Result<()> {
    let lua = create_lua()?;

    // Test ::1 address defaults to local connection
    let host_table = lua.create_table()?;
    host_table.set("address", "::1")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {
            // Success - ::1 should create local connection
        }
        Connection::SSH(_) => {
            panic!("Expected local connection for ::1 address");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_explicit_local() -> Result<()> {
    let lua = create_lua()?;

    // Test explicit local connection type overrides remote address
    let host_table = lua.create_table()?;
    host_table.set("address", "invalid.host")?;
    host_table.set("connection", "local")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::Local(_) => {
            // Success - explicit local should override remote address
        }
        Connection::SSH(_) => {
            panic!("Expected local connection when explicitly set to local");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_explicit_ssh() -> Result<()> {
    let lua = create_lua()?;

    // Test explicit SSH connection type overrides localhost
    let host_table = lua.create_table()?;
    host_table.set("address", "test.rebex.net")?;
    host_table.set("connection", "ssh")?;
    host_table.set("user", "demo")?;
    host_table.set("password", "password")?;
    host_table.set("host_key_check", false)?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::SSH(_) => {
            // Success - explicit SSH should override localhost
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection when explicitly set to ssh");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_remote_address_defaults_ssh() -> Result<()> {
    let lua = create_lua()?;

    // Test remote address defaults to SSH connection
    let host_table = lua.create_table()?;
    host_table.set("address", "test.rebex.net")?;
    host_table.set("user", "demo")?;
    host_table.set("password", "password")?;
    host_table.set("host_key_check", false)?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::SSH(_) => {
            // Success - remote address should default to SSH
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for remote address");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_with_environment_variables() -> Result<()> {
    let lua = create_lua()?;

    // Test connection with environment variables
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let env_table = lua.create_table()?;
    env_table.set("TEST_VAR", "test_value")?;
    env_table.set("ANOTHER_VAR", "another_value")?;
    host_table.set("env", env_table)?;

    let mut connection = create_connection(&lua, &Value::Table(host_table))?;

    match &mut connection {
        Connection::Local(local) => {
            // Test that environment variables are set
            local.set_env("VERIFY_VAR", "verify_value");
            // We can't directly verify the env vars were set from the host config
            // but we can verify the connection was created successfully
        }
        Connection::SSH(_) => {
            panic!("Expected local connection for localhost");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_ssh_with_key_authentication() -> Result<()> {
    let lua = create_lua()?;

    // Test SSH connection with key authentication
    let host_table = lua.create_table()?;
    host_table.set("address", "127.0.0.1")?;
    host_table.set("connection", "ssh")?;
    host_table.set("user", "usertest")?;
    let home = env::var("HOME")?;
    let key_path = format!("{home}/.ssh/id_ed25519");
    host_table.set("private_key_file", key_path)?;
    host_table.set("host_key_check", false)?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::SSH(_) => {
            // Success - SSH connection with key auth should be created
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for remote address with key auth");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_ssh_with_custom_port() -> Result<()> {
    let lua = create_lua()?;

    // Test SSH connection with custom port
    let host_table = lua.create_table()?;
    host_table.set("address", "test.rebex.net")?;
    host_table.set("port", 22)?;
    host_table.set("user", "demo")?;
    host_table.set("password", "password")?;
    host_table.set("host_key_check", false)?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::SSH(_) => {
            // Success - SSH connection with custom port should be created
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for remote address");
        }
    }

    Ok(())
}

#[test]
fn test_connection_factory_with_defaults() -> Result<()> {
    let lua = create_lua()?;

    // Set some defaults
    lua.load(mlua::chunk! {
        komandan.defaults:set_user("demo")
        komandan.defaults:set_port(22)
        komandan.defaults:set_host_key_check(false)
    })
    .exec()?;

    // Test that defaults are applied
    let host_table = lua.create_table()?;
    host_table.set("address", "test.rebex.net")?;
    host_table.set("password", "password")?;

    let connection = create_connection(&lua, &Value::Table(host_table))?;

    match connection {
        Connection::SSH(_) => {
            // Success - SSH connection should use defaults
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for remote address");
        }
    }

    // Reset defaults to avoid affecting other tests
    lua.load(mlua::chunk! {
        komandan.defaults:set_user(nil)
        komandan.defaults:set_port(22)
        komandan.defaults:set_host_key_check(true)
    })
    .exec()?;

    Ok(())
}

#[test]
fn test_connection_factory_error_handling_invalid_host() -> Result<()> {
    let lua = create_lua()?;

    // Test error handling for invalid host configuration
    let invalid_host = lua.create_table()?;
    // Missing required address field

    let result = create_connection(&lua, &Value::Table(invalid_host));

    // Should return an error for invalid host configuration
    assert!(result.is_err());

    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("Host validation failed") || error_msg.contains("address"));
    }

    Ok(())
}

#[test]
fn test_connection_factory_command_execution_interface() -> Result<()> {
    let lua = create_lua()?;

    // Test that the connection provides a consistent command execution interface
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let mut connection = create_connection(&lua, &Value::Table(host_table))?;

    // Test command execution interface
    let (stdout, stderr, exit_code) = connection.cmd("echo test")?;
    assert_eq!(stdout, "test");
    assert_eq!(stderr, "");
    assert_eq!(exit_code, 0);

    // Test quiet command execution interface
    let (stdout, stderr, exit_code) = connection.cmdq("echo quiet")?;
    assert_eq!(stdout, "quiet");
    assert_eq!(stderr, "");
    assert_eq!(exit_code, 0);

    Ok(())
}

#[test]
fn test_connection_factory_environment_interface() -> Result<()> {
    let lua = create_lua()?;

    // Test that the connection provides environment variable interface
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let mut connection = create_connection(&lua, &Value::Table(host_table))?;

    // Test environment variable setting
    connection.set_env("TEST_KEY", "test_value");

    // Verify environment variable is set by executing a command that uses it
    let (stdout, stderr, exit_code) = connection.cmd("echo $TEST_KEY")?;
    assert_eq!(stdout, "test_value");
    assert_eq!(stderr, "");
    assert_eq!(exit_code, 0);

    Ok(())
}

#[test]
fn test_connection_factory_connection_type_detection() -> Result<()> {
    let lua = create_lua()?;

    // Test local connection type detection
    let local_host = lua.create_table()?;
    local_host.set("address", "localhost")?;

    let local_connection = create_connection(&lua, &Value::Table(local_host))?;
    assert_eq!(
        local_connection.connection_type(),
        komandan::models::ConnectionType::Local
    );

    // Test SSH connection type detection
    let ssh_host = lua.create_table()?;
    ssh_host.set("address", "test.rebex.net")?;
    ssh_host.set("user", "demo")?;
    ssh_host.set("password", "password")?;
    ssh_host.set("host_key_check", false)?;

    let ssh_connection = create_connection(&lua, &Value::Table(ssh_host))?;
    assert_eq!(
        ssh_connection.connection_type(),
        komandan::models::ConnectionType::SSH
    );

    Ok(())
}

/// Integration test with real SSH server (requires SSH test environment)
/// This test is skipped unless `KOMANDAN_SSH_TEST` environment variable is set
#[test]
fn test_connection_factory_real_ssh_integration() -> Result<()> {
    // Skip if SSH test environment not available
    if env::var("KOMANDAN_SSH_TEST").is_err() {
        eprintln!("Skipping real SSH integration test - set KOMANDAN_SSH_TEST=1 to enable");
        return Ok(());
    }

    let lua = create_lua()?;

    // Create SSH connection to test server
    let host_table = lua.create_table()?;
    host_table.set("address", "127.0.0.1")?;
    host_table.set("connection", "ssh")?; // Force SSH even for localhost
    host_table.set("user", "usertest")?;

    // Use SSH key authentication
    let home = env::var("HOME")?;
    let key_path = format!("{home}/.ssh/id_ed25519");
    host_table.set("private_key_file", key_path)?;
    host_table.set("host_key_check", false)?;

    let mut connection = create_connection(&lua, &Value::Table(host_table))?;

    match &mut connection {
        Connection::SSH(_) => {
            // Test basic command execution
            let (stdout, stderr, exit_code) = connection.cmd("echo 'SSH integration test'")?;
            assert_eq!(exit_code, 0);
            assert_eq!(stdout, "SSH integration test");
            assert_eq!(stderr, "");

            // Test environment variables
            connection.set_env("INTEGRATION_TEST", "true");
            let (stdout, stderr, exit_code) = connection.cmd("echo $INTEGRATION_TEST")?;
            assert_eq!(exit_code, 0);
            assert_eq!(stdout, "true");
            assert_eq!(stderr, "");
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for explicit SSH configuration");
        }
    }

    Ok(())
}

/// Test that connection factory maintains backward compatibility
#[test]
fn test_connection_factory_backward_compatibility() -> Result<()> {
    let lua = create_lua()?;

    // Test that existing host configurations still work
    let legacy_host = lua.create_table()?;
    legacy_host.set("name", "legacy-server")?;
    legacy_host.set("address", "test.rebex.net")?;
    legacy_host.set("user", "demo")?;
    legacy_host.set("password", "password")?;
    legacy_host.set("port", 22)?;
    legacy_host.set("host_key_check", false)?;

    // Add some legacy environment variables
    let env_table = lua.create_table()?;
    env_table.set("DEPLOY_ENV", "production")?;
    env_table.set("APP_VERSION", "1.0.0")?;
    legacy_host.set("env", env_table)?;

    let connection = create_connection(&lua, &Value::Table(legacy_host))?;

    match connection {
        Connection::SSH(_) => {
            // Success - legacy configuration should work
        }
        Connection::Local(_) => {
            panic!("Expected SSH connection for legacy remote host");
        }
    }

    Ok(())
}
