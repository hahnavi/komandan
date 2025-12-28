use crate::connection::Connection;
use crate::executor::CommandExecutor;
use crate::local::LocalSession;
use crate::ssh::{Elevation, ElevationMethod, SSHAuthMethod, SSHSession};
use anyhow::Result;
use mlua::{Lua, Table, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Type alias for command response tuple (stdout, stderr, `exit_code`)
type CommandResponse = (String, String, i32);

/// Type alias for connection parameters tuple
type ConnectionParams = (String, u16, String, SSHAuthMethod);

/// Mock SSH session for testing that implements the same interface as `SSHSession`
#[derive(Clone)]
pub struct MockSSHSession {
    /// Mock command responses - maps command patterns to (stdout, stderr, `exit_code`)
    pub command_responses: Arc<Mutex<HashMap<String, CommandResponse>>>,
    /// Environment variables set on the session
    pub env: HashMap<String, String>,
    /// Elevation configuration
    pub elevation: Elevation,
    /// Known hosts file setting
    pub known_hosts_file: Option<String>,
    /// Connection parameters for verification
    pub connection_params: Arc<Mutex<Option<ConnectionParams>>>,
    /// Whether the session should simulate being connected
    pub connected: Arc<Mutex<bool>>,
    /// Commands that have been executed (for verification)
    pub executed_commands: Arc<Mutex<Vec<String>>>,
    /// Session state for tracking
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
    changed: Option<bool>,
}

impl MockSSHSession {
    /// Create a new mock SSH session
    ///
    /// # Errors
    /// Returns an error if the mock session cannot be initialized
    pub fn new() -> Result<Self> {
        Ok(Self {
            command_responses: Arc::new(Mutex::new(HashMap::new())),
            env: HashMap::new(),
            elevation: Elevation {
                method: ElevationMethod::None,
                as_user: None,
            },
            known_hosts_file: None,
            connection_params: Arc::new(Mutex::new(None)),
            connected: Arc::new(Mutex::new(false)),
            executed_commands: Arc::new(Mutex::new(Vec::new())),
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            exit_code: Some(0),
            changed: Some(false),
        })
    }

    /// Set a mock response for a specific command pattern
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn set_command_response(
        &self,
        pattern: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> Result<()> {
        self.command_responses
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?
            .insert(
                pattern.to_string(),
                (stdout.to_string(), stderr.to_string(), exit_code),
            );
        Ok(())
    }

    /// Set multiple command responses at once
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn set_command_responses(&self, responses: Vec<(&str, &str, &str, i32)>) -> Result<()> {
        let mut response_map = self
            .command_responses
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
        for (pattern, stdout, stderr, exit_code) in responses {
            response_map.insert(
                pattern.to_string(),
                (stdout.to_string(), stderr.to_string(), exit_code),
            );
        }
        drop(response_map);
        Ok(())
    }

    /// Simulate connection success
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn simulate_connection_success(&self) -> Result<()> {
        *self
            .connected
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))? = true;
        Ok(())
    }

    /// Simulate connection failure with specific error
    ///
    /// # Errors
    /// Returns an error based on the specified error type or if mutex is poisoned
    pub fn simulate_connection_failure(&self, error_type: &MockConnectionError) -> Result<()> {
        *self
            .connected
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))? = false;
        match error_type {
            MockConnectionError::Authentication => {
                Err(anyhow::anyhow!("SSH authentication failed"))
            }
            MockConnectionError::HostKeyVerification => {
                Err(anyhow::anyhow!("SSH host key verification failed"))
            }
            MockConnectionError::ConnectionRefused => Err(anyhow::anyhow!("Connection refused")),
            MockConnectionError::Timeout => Err(anyhow::anyhow!("Connection timed out")),
            MockConnectionError::NetworkUnreachable => Err(anyhow::anyhow!("Network unreachable")),
        }
    }

    /// Get the commands that have been executed
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn get_executed_commands(&self) -> Result<Vec<String>> {
        Ok(self
            .executed_commands
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?
            .clone())
    }

    /// Get the connection parameters that were used
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn get_connection_params(&self) -> Result<Option<ConnectionParams>> {
        Ok(self
            .connection_params
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?
            .clone())
    }

    /// Check if the session is connected
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn is_connected(&self) -> Result<bool> {
        Ok(*self
            .connected
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?)
    }

    /// Connect to an SSH server (mock implementation)
    ///
    /// # Errors
    /// Returns an error if the mock connection is configured to fail or if mutex is poisoned
    pub fn connect(
        &mut self,
        address: &str,
        port: u16,
        username: &str,
        auth_method: SSHAuthMethod,
    ) -> Result<()> {
        // Store connection parameters for verification
        {
            let mut params = self
                .connection_params
                .lock()
                .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
            *params = Some((address.to_string(), port, username.to_string(), auth_method));
        }

        // Check if we should simulate connection failure
        if !*self
            .connected
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?
        {
            return Err(anyhow::anyhow!("Mock connection failure"));
        }

        Ok(())
    }

    /// Find matching command response
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    fn find_command_response(&self, command: &str) -> Result<CommandResponse> {
        let responses = self
            .command_responses
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        // First try exact match
        if let Some(response) = responses.get(command) {
            return Ok(response.clone());
        }

        // Then try pattern matching
        for (pattern, response) in responses.iter() {
            if command.contains(pattern) {
                return Ok(response.clone());
            }
        }
        drop(responses);

        // Default response if no match found
        Ok((String::new(), String::new(), 0))
    }
}

impl CommandExecutor for MockSSHSession {
    fn cmd(&mut self, command: &str) -> Result<(String, String, i32)> {
        // Record the command
        {
            let mut commands = self
                .executed_commands
                .lock()
                .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
            commands.push(command.to_string());
        }

        // Find response
        let (stdout, stderr, exit_code) = self.find_command_response(command)?;

        // Update session state
        if let Some(stdout_buf) = self.stdout.as_mut() {
            stdout_buf.push_str(&stdout);
        }
        if let Some(stderr_buf) = self.stderr.as_mut() {
            stderr_buf.push_str(&stderr);
        }
        self.exit_code = Some(exit_code);

        Ok((stdout, stderr, exit_code))
    }

    fn cmdq(&self, command: &str) -> Result<(String, String, i32)> {
        // Record the command
        {
            let mut commands = self
                .executed_commands
                .lock()
                .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;
            commands.push(command.to_string());
        }

        // Find response
        let (stdout, stderr, exit_code) = self.find_command_response(command)?;

        Ok((stdout, stderr, exit_code))
    }

    fn prepare_command(&self, command: &str) -> String {
        match self.elevation.method {
            ElevationMethod::Su => self.elevation.as_user.as_ref().map_or_else(
                || format!("su -c '{command}'"),
                |user| format!("su {user} -c '{command}'"),
            ),
            ElevationMethod::Sudo => self.elevation.as_user.as_ref().map_or_else(
                || format!("sudo -E {command}"),
                |user| format!("sudo -E -u {user} {command}"),
            ),
            ElevationMethod::None => command.to_string(),
        }
    }

    fn set_env(&mut self, key: &str, value: &str) {
        self.env.insert(key.to_string(), value.to_string());
    }

    fn get_remote_env(&self, var: &str) -> Result<String> {
        Ok(self.env.get(var).cloned().unwrap_or_default())
    }

    fn get_tmpdir(&self) -> Result<String> {
        Ok("/tmp/komandan".to_string())
    }

    fn upload(&self, _local_path: &std::path::Path, _remote_path: &std::path::Path) -> Result<()> {
        Ok(())
    }

    fn download(
        &self,
        _remote_path: &std::path::Path,
        _local_path: &std::path::Path,
    ) -> Result<()> {
        Ok(())
    }

    fn write_remote_file(&self, _remote_path: &std::path::Path, _content: &[u8]) -> Result<()> {
        Ok(())
    }

    fn chmod(&self, _remote_path: &std::path::Path, _mode: &str) -> Result<()> {
        Ok(())
    }

    fn set_changed(&mut self, changed: bool) {
        self.changed = Some(changed);
    }

    fn get_changed(&self) -> bool {
        self.changed.unwrap_or(false)
    }

    fn get_session_result(&self) -> crate::executor::SessionResult {
        crate::executor::SessionResult {
            stdout: self.stdout.as_ref().unwrap_or(&String::new()).clone(),
            stderr: self.stderr.as_ref().unwrap_or(&String::new()).clone(),
            exit_code: self.exit_code.unwrap_or(-1),
            changed: self.changed.unwrap_or(false),
        }
    }
}

/// Types of connection errors that can be simulated
#[derive(Debug, Clone, Copy)]
pub enum MockConnectionError {
    Authentication,
    HostKeyVerification,
    ConnectionRefused,
    Timeout,
    NetworkUnreachable,
}

/// Mock connection factory for testing
pub struct MockConnectionFactory {
    /// Mock responses for different host configurations
    pub host_responses: Arc<Mutex<HashMap<String, MockConnectionResponse>>>,
}

/// Response configuration for mock connections
#[derive(Clone)]
pub struct MockConnectionResponse {
    pub connection_type: crate::models::ConnectionType,
    pub should_fail: bool,
    pub failure_error: Option<MockConnectionError>,
    pub ssh_session: Option<MockSSHSession>,
}

impl MockConnectionFactory {
    /// Create a new mock connection factory
    #[must_use]
    pub fn new() -> Self {
        Self {
            host_responses: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set mock response for a specific host address
    ///
    /// # Errors
    /// Returns an error if the internal mutex is poisoned
    pub fn set_host_response(&self, address: &str, response: MockConnectionResponse) -> Result<()> {
        self.host_responses
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?
            .insert(address.to_string(), response);
        Ok(())
    }

    /// Create a mock connection based on host configuration
    ///
    /// # Errors
    /// Returns an error if the host configuration is invalid or connection fails
    pub fn create_mock_connection(&self, _lua: &Lua, host: Value) -> mlua::Result<Connection> {
        let Value::Table(host_table) = host else {
            return Err(mlua::Error::RuntimeError(
                "Host must be a table".to_string(),
            ));
        };

        let address = host_table
            .get::<String>("address")
            .map_err(|_| mlua::Error::RuntimeError("Host address is required".to_string()))?;

        let response = {
            let responses = self
                .host_responses
                .lock()
                .map_err(|_| mlua::Error::RuntimeError("Mutex poisoned".to_string()))?;
            responses
                .get(&address)
                .cloned()
                .unwrap_or(MockConnectionResponse {
                    connection_type: crate::models::ConnectionType::Local,
                    should_fail: false,
                    failure_error: None,
                    ssh_session: None,
                })
        };

        if response.should_fail {
            let error_msg = match response.failure_error {
                Some(MockConnectionError::Authentication) => "Mock authentication failure",
                Some(MockConnectionError::HostKeyVerification) => {
                    "Mock host key verification failure"
                }
                Some(MockConnectionError::ConnectionRefused) => "Mock connection refused",
                Some(MockConnectionError::Timeout) => "Mock connection timeout",
                Some(MockConnectionError::NetworkUnreachable) => "Mock network unreachable",
                None => "Mock connection failure",
            };
            return Err(mlua::Error::RuntimeError(error_msg.to_string()));
        }

        match response.connection_type {
            crate::models::ConnectionType::Local => {
                let local = LocalSession::new();
                Ok(Connection::Local(local))
            }
            crate::models::ConnectionType::SSH => {
                if let Some(_mock_ssh) = response.ssh_session {
                    // Convert MockSSHSession to SSHSession for the Connection enum
                    // For testing purposes, we'll create a real SSHSession but not connect it
                    let ssh = SSHSession::new().map_err(|e| {
                        mlua::Error::RuntimeError(format!("Failed to create SSH session: {e}"))
                    })?;
                    Ok(Connection::SSH(ssh))
                } else {
                    let ssh = SSHSession::new().map_err(|e| {
                        mlua::Error::RuntimeError(format!("Failed to create SSH session: {e}"))
                    })?;
                    Ok(Connection::SSH(ssh))
                }
            }
        }
    }
}

impl Default for MockConnectionFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// Test utilities for creating common test scenarios
pub struct TestUtils;

impl TestUtils {
    /// Create a mock SSH session with common command responses
    ///
    /// # Errors
    /// Returns an error if the mock session cannot be created
    pub fn create_mock_ssh_session() -> Result<MockSSHSession> {
        let mock = MockSSHSession::new()?;

        // Set up common command responses
        mock.set_command_responses(vec![
            ("echo test", "test", "", 0),
            ("whoami", "testuser", "", 0),
            ("pwd", "/home/testuser", "", 0),
            ("uname -r", "5.4.0-42-generic", "", 0),
            ("hostname", "testhost", "", 0),
            ("id -u", "1000", "", 0),
            ("id -g", "1000", "", 0),
            (
                "getent passwd",
                "testuser:x:1000:1000:Test User:/home/testuser:/bin/bash",
                "",
                0,
            ),
        ])?;

        mock.simulate_connection_success()?;
        Ok(mock)
    }

    /// Create a mock SSH session that simulates authentication failure
    ///
    /// # Errors
    /// Returns an error if the mock session cannot be created
    pub fn create_failing_auth_ssh_session() -> Result<MockSSHSession> {
        // Don't call simulate_connection_success() to keep it in failed state
        MockSSHSession::new()
    }

    /// Create a mock SSH session with custom command responses
    ///
    /// # Errors
    /// Returns an error if the mock session cannot be created
    pub fn create_custom_mock_ssh_session(
        responses: Vec<(&str, &str, &str, i32)>,
    ) -> Result<MockSSHSession> {
        let mock = MockSSHSession::new()?;
        mock.set_command_responses(responses)?;
        mock.simulate_connection_success()?;
        Ok(mock)
    }

    /// Create a test host configuration for SSH
    ///
    /// # Errors
    /// Returns an error if the Lua table cannot be created or configured
    pub fn create_test_ssh_host(lua: &Lua) -> mlua::Result<Table> {
        let host = lua.create_table()?;
        host.set("name", "test-host")?;
        host.set("address", "test.example.com")?;
        host.set("user", "testuser")?;
        host.set("password", "testpass")?;
        host.set("host_key_check", false)?;
        Ok(host)
    }

    /// Create a test host configuration for local connection
    ///
    /// # Errors
    /// Returns an error if the Lua table cannot be created or configured
    pub fn create_test_local_host(lua: &Lua) -> mlua::Result<Table> {
        let host = lua.create_table()?;
        host.set("name", "local-host")?;
        host.set("address", "localhost")?;
        Ok(host)
    }

    /// Create a test host configuration with SSH key authentication
    ///
    /// # Errors
    /// Returns an error if the Lua table cannot be created or configured
    pub fn create_test_ssh_key_host(lua: &Lua) -> mlua::Result<Table> {
        let host = lua.create_table()?;
        host.set("name", "key-host")?;
        host.set("address", "key.example.com")?;
        host.set("user", "testuser")?;
        host.set("private_key_file", "/path/to/test/key")?;
        host.set("host_key_check", false)?;
        Ok(host)
    }

    /// Create a dummy task for testing
    ///
    /// # Errors
    /// Returns an error if the Lua table cannot be created
    pub fn create_dummy_task(lua: &Lua) -> mlua::Result<Table> {
        lua.create_table()
    }

    /// Create a test task with command module
    ///
    /// # Errors
    /// Returns an error if the Lua table cannot be created or configured
    pub fn create_test_cmd_task(lua: &Lua, command: &str) -> mlua::Result<Table> {
        let task = lua.create_table()?;
        task.set("name", "Test command")?;

        let module_params = lua.create_table()?;
        module_params.set("cmd", command)?;

        // Create the module function call
        let module = lua
            .load(mlua::chunk! {
                return komandan.modules.cmd($module_params)
            })
            .eval::<Table>()?;

        task.set(1, module)?;
        Ok(task)
    }

    /// Create a mock connection factory with common test scenarios
    ///
    /// # Errors
    /// Returns an error if the factory cannot be configured
    pub fn create_test_connection_factory() -> Result<MockConnectionFactory> {
        let factory = MockConnectionFactory::new();

        // Set up localhost to use local connection
        factory.set_host_response(
            "localhost",
            MockConnectionResponse {
                connection_type: crate::models::ConnectionType::Local,
                should_fail: false,
                failure_error: None,
                ssh_session: None,
            },
        )?;

        // Set up a working SSH host
        factory.set_host_response(
            "test.example.com",
            MockConnectionResponse {
                connection_type: crate::models::ConnectionType::SSH,
                should_fail: false,
                failure_error: None,
                ssh_session: Some(Self::create_mock_ssh_session()?),
            },
        )?;

        // Set up a failing SSH host
        factory.set_host_response(
            "fail.example.com",
            MockConnectionResponse {
                connection_type: crate::models::ConnectionType::SSH,
                should_fail: true,
                failure_error: Some(MockConnectionError::Authentication),
                ssh_session: None,
            },
        )?;

        Ok(factory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_mock_ssh_session_creation() -> Result<()> {
        let mock = MockSSHSession::new()?;
        assert!(!mock.is_connected()?);
        assert_eq!(mock.get_executed_commands()?.len(), 0);
        Ok(())
    }

    #[test]
    fn test_mock_ssh_session_command_responses() -> Result<()> {
        let mut mock = MockSSHSession::new()?;
        mock.set_command_response("echo test", "test output", "", 0)?;

        let (stdout, stderr, exit_code) = mock.cmd("echo test")?;
        assert_eq!(stdout, "test output");
        assert_eq!(stderr, "");
        assert_eq!(exit_code, 0);

        let commands = mock.get_executed_commands()?;
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], "echo test");

        Ok(())
    }

    #[test]
    fn test_mock_ssh_session_connection_simulation() -> Result<()> {
        let mut mock = MockSSHSession::new()?;

        // Test connection failure
        assert!(!mock.is_connected()?);
        let result = mock.connect(
            "test.com",
            22,
            "user",
            SSHAuthMethod::Password("pass".to_string()),
        );
        assert!(result.is_err());

        // Test connection success
        mock.simulate_connection_success()?;
        assert!(mock.is_connected()?);
        let result = mock.connect(
            "test.com",
            22,
            "user",
            SSHAuthMethod::Password("pass".to_string()),
        );
        assert!(result.is_ok());

        // Verify connection parameters
        let params = mock
            .get_connection_params()?
            .ok_or_else(|| anyhow::anyhow!("Connection params should be set"))?;
        assert_eq!(params.0, "test.com");
        assert_eq!(params.1, 22);
        assert_eq!(params.2, "user");

        Ok(())
    }

    #[test]
    fn test_mock_connection_factory() -> mlua::Result<()> {
        let lua = create_lua()?;
        let factory = MockConnectionFactory::new();

        // Set up a local host response
        factory
            .set_host_response(
                "localhost",
                MockConnectionResponse {
                    connection_type: crate::models::ConnectionType::Local,
                    should_fail: false,
                    failure_error: None,
                    ssh_session: None,
                },
            )
            .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        let host = lua.create_table()?;
        host.set("address", "localhost")?;

        let connection = factory.create_mock_connection(&lua, Value::Table(host))?;
        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection"),
        }

        Ok(())
    }

    #[test]
    fn test_test_utils_create_hosts() -> mlua::Result<()> {
        let lua = create_lua()?;

        let ssh_host = TestUtils::create_test_ssh_host(&lua)?;
        assert_eq!(ssh_host.get::<String>("address")?, "test.example.com");
        assert_eq!(ssh_host.get::<String>("user")?, "testuser");

        let local_host = TestUtils::create_test_local_host(&lua)?;
        assert_eq!(local_host.get::<String>("address")?, "localhost");

        let key_host = TestUtils::create_test_ssh_key_host(&lua)?;
        assert_eq!(
            key_host.get::<String>("private_key_file")?,
            "/path/to/test/key"
        );

        Ok(())
    }

    #[test]
    fn test_test_utils_create_tasks() -> mlua::Result<()> {
        let lua = create_lua()?;

        let dummy_task = TestUtils::create_dummy_task(&lua)?;
        assert_eq!(dummy_task.len()?, 0);

        let cmd_task = TestUtils::create_test_cmd_task(&lua, "echo test")?;
        assert_eq!(cmd_task.get::<String>("name")?, "Test command");

        Ok(())
    }

    #[test]
    fn test_mock_ssh_session_elevation() -> Result<()> {
        let mock = MockSSHSession::new()?;

        // Test no elevation
        let cmd = mock.prepare_command("ls -la");
        assert_eq!(cmd, "ls -la");

        // Test sudo elevation
        let mut mock_sudo = mock;
        mock_sudo.elevation.method = ElevationMethod::Sudo;
        let cmd = mock_sudo.prepare_command("ls -la");
        assert_eq!(cmd, "sudo -E ls -la");

        // Test su elevation
        let mut mock_su = MockSSHSession::new()?;
        mock_su.elevation.method = ElevationMethod::Su;
        let cmd = mock_su.prepare_command("ls -la");
        assert_eq!(cmd, "su -c 'ls -la'");

        Ok(())
    }

    #[test]
    fn test_mock_ssh_session_environment() -> Result<()> {
        let mut mock = MockSSHSession::new()?;
        mock.set_env("TEST_VAR", "test_value");

        let value = mock.get_remote_env("TEST_VAR")?;
        assert_eq!(value, "test_value");

        let empty_value = mock.get_remote_env("NONEXISTENT")?;
        assert_eq!(empty_value, "");

        Ok(())
    }
}
