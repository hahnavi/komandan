use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Result of a command execution session
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub changed: bool,
}

/// Trait for command execution, implemented by both SSH and local sessions
pub trait CommandExecutor {
    /// Execute a command and track the output in the session
    ///
    /// # Errors
    ///
    /// Returns an error if the command execution fails or if there are issues reading the output.
    fn cmd(&mut self, command: &str) -> Result<(String, String, i32)>;

    /// Execute a command quietly (without tracking in session)
    ///
    /// # Errors
    ///
    /// Returns an error if the command execution fails or if there are issues reading the output.
    fn cmdq(&self, command: &str) -> Result<(String, String, i32)>;

    /// Prepare a command with elevation if needed
    fn prepare_command(&self, command: &str) -> String;

    /// Set an environment variable for command execution
    fn set_env(&mut self, key: &str, value: &str);

    /// Get an environment variable from the remote/local system
    ///
    /// # Errors
    ///
    /// Returns an error if the command to retrieve the environment variable fails.
    fn get_remote_env(&self, var: &str) -> Result<String>;

    /// Get a temporary directory path
    ///
    /// # Errors
    ///
    /// Returns an error if it fails to find or create a temporary directory.
    fn get_tmpdir(&self) -> Result<String>;

    /// Upload a file or directory from local to remote/target
    ///
    /// # Errors
    ///
    /// Returns an error if the upload fails, e.g., due to network issues or permission errors.
    fn upload(&self, local_path: &Path, remote_path: &Path) -> Result<()>;

    /// Download a file or directory from remote/target to local
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails, e.g., due to network issues or permission errors.
    fn download(&self, remote_path: &Path, local_path: &Path) -> Result<()>;

    /// Write content to a remote/target file
    ///
    /// # Errors
    ///
    /// Returns an error if the write operation fails.
    fn write_remote_file(&self, remote_path: &Path, content: &[u8]) -> Result<()>;

    /// Change file permissions
    ///
    /// # Errors
    ///
    /// Returns an error if the chmod command fails.
    fn chmod(&self, remote_path: &Path, mode: &str) -> Result<()>;

    /// Set the changed flag for this session
    fn set_changed(&mut self, changed: bool);

    /// Get the changed flag for this session
    fn get_changed(&self) -> bool;

    /// Get the complete session result
    fn get_session_result(&self) -> SessionResult;
}
