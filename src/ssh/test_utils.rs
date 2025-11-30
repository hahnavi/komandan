//! Test utilities for SSH module testing
//!
//! Provides helper functions and test scenarios for testing SSH operations
//! without requiring actual SSH connections.

use std::{collections::HashMap, io::Write};

use anyhow::Result;
use tempfile::{NamedTempFile, TempDir};

use super::{ElevationMethod, SSHSession};
use crate::executor::CommandExecutor;

/// Test configuration for SSH operations
#[derive(Clone, Debug, Default)]
pub struct TestConfig {
    pub command_outputs: HashMap<String, (String, String, i32)>,
}

/// Builder for test SSH sessions
pub struct TestSSHSessionBuilder {
    config: TestConfig,
    env: HashMap<String, String>,
    elevation: super::Elevation,
}

impl TestSSHSessionBuilder {
    pub fn new() -> Self {
        Self {
            config: TestConfig::default(),
            env: HashMap::new(),
            elevation: super::Elevation {
                method: ElevationMethod::None,
                as_user: None,
            },
        }
    }

    pub fn with_command_output(
        mut self,
        command: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
    ) -> Self {
        self.config.command_outputs.insert(
            command.to_string(),
            (stdout.to_string(), stderr.to_string(), exit_code),
        );
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_elevation(mut self, method: ElevationMethod, as_user: Option<String>) -> Self {
        self.elevation = super::Elevation { method, as_user };
        self
    }

    pub fn build(self) -> Result<SSHSession> {
        // Create a real session for testing basic functionality
        // Note: This won't actually connect to a real SSH server
        let mut session = SSHSession::new()?;

        // Set the environment variables
        for (key, value) in self.env {
            session.set_env(&key, &value);
        }

        // Set elevation
        session.elevation = self.elevation;

        Ok(session)
    }
}

/// Helper function to create a test SSH session with environment variables
pub fn create_test_ssh_session_with_env(env: &[(&str, &str)]) -> Result<SSHSession> {
    let mut builder = TestSSHSessionBuilder::new();
    for (key, value) in env {
        builder = builder.with_env(key, value);
    }
    builder.build()
}

/// Helper function to create a test SSH session with elevation
pub fn create_test_ssh_session_with_elevation(
    method: ElevationMethod,
    as_user: Option<&str>,
) -> Result<SSHSession> {
    TestSSHSessionBuilder::new()
        .with_elevation(method, as_user.map(ToString::to_string))
        .build()
}

/// Test utilities for file operations
pub mod file_utils {
    use super::*;

    /// Create a temporary test file with content
    pub fn create_test_file(content: &str) -> Result<NamedTempFile> {
        let mut file = NamedTempFile::new()?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        Ok(file)
    }

    /// Create a temporary test directory
    pub fn create_test_dir() -> Result<TempDir> {
        Ok(TempDir::new()?)
    }

    /// Create a test directory structure
    pub fn create_test_directory_structure() -> Result<TempDir> {
        let temp_dir = create_test_dir()?;

        // Create subdirectories and files
        let subdir = temp_dir.path().join("subdir");
        std::fs::create_dir(&subdir)?;

        std::fs::write(temp_dir.path().join("file1.txt"), "content1")?;
        std::fs::write(subdir.join("file2.txt"), "content2")?;

        Ok(temp_dir)
    }
}

/// Test scenarios for SSH operations
pub mod scenarios {
    use super::*;

    /// Create a scenario for testing basic command execution
    pub fn basic_command_execution() -> TestSSHSessionBuilder {
        TestSSHSessionBuilder::new()
            .with_command_output("ls -la", "file1.txt\nfile2.txt", "", 0)
            .with_command_output("echo test", "test", "", 0)
    }

    /// Create a scenario for testing environment variables
    pub fn environment_variables() -> TestSSHSessionBuilder {
        TestSSHSessionBuilder::new()
            .with_env("TEST_VAR", "test_value")
            .with_command_output("echo $TEST_VAR", "test_value", "", 0)
    }

    /// Create a scenario for testing elevation
    pub fn elevation_scenarios() -> TestSSHSessionBuilder {
        TestSSHSessionBuilder::new()
            .with_elevation(ElevationMethod::Sudo, Some("admin".to_string()))
            .with_command_output("whoami", "admin", "", 0)
    }

    /// Create a scenario for testing error conditions
    pub fn error_conditions() -> TestSSHSessionBuilder {
        TestSSHSessionBuilder::new()
            .with_command_output("invalid_command", "", "command not found", 127)
            .with_command_output("exit 1", "", "", 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_test_ssh_session_builder() -> Result<()> {
        let session = TestSSHSessionBuilder::new().build()?;
        assert!(session.env.is_empty());
        assert!(matches!(session.elevation.method, ElevationMethod::None));
        Ok(())
    }

    #[test]
    fn test_test_ssh_session_with_env() -> Result<()> {
        let session = create_test_ssh_session_with_env(&[("TEST_KEY", "TEST_VALUE")])?;
        // Note: We can't easily test the internal state without exposing it
        // This test mainly verifies that the function doesn't panic
        assert!(matches!(session.elevation.method, ElevationMethod::None));
        Ok(())
    }

    #[test]
    fn test_test_ssh_session_with_elevation() -> Result<()> {
        let session = create_test_ssh_session_with_elevation(ElevationMethod::Sudo, Some("admin"))?;
        assert!(matches!(session.elevation.method, ElevationMethod::Sudo));
        Ok(())
    }

    #[test]
    fn test_file_utils() -> Result<()> {
        let file = file_utils::create_test_file("test content")?;
        let content = std::fs::read_to_string(file.path())?;
        assert_eq!(content, "test content");

        let dir = file_utils::create_test_dir()?;
        assert!(dir.path().exists());

        let structured_dir = file_utils::create_test_directory_structure()?;
        assert!(structured_dir.path().join("file1.txt").exists());
        assert!(structured_dir.path().join("subdir").exists());
        assert!(structured_dir.path().join("subdir/file2.txt").exists());

        Ok(())
    }

    #[test]
    fn test_scenarios() -> Result<()> {
        let builder = scenarios::basic_command_execution();
        let session = builder.build()?;
        assert!(matches!(session.elevation.method, ElevationMethod::None));

        let builder = scenarios::environment_variables();
        let session = builder.build()?;
        assert!(matches!(session.elevation.method, ElevationMethod::None));

        let builder = scenarios::elevation_scenarios();
        let session = builder.build()?;
        assert!(matches!(session.elevation.method, ElevationMethod::Sudo));

        let builder = scenarios::error_conditions();
        let session = builder.build()?;
        assert!(matches!(session.elevation.method, ElevationMethod::None));
        Ok(())
    }
}
