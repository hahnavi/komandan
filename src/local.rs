use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Error, Result};
use mlua::{Error::RuntimeError, UserData, Value};

use crate::executor::{CommandExecutor, SessionResult};
use crate::ssh::{Elevation, ElevationMethod};

use std::sync::LazyLock;

use regex::Regex;

fn escape_shell_value(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn is_valid_env_var_name(name: &str) -> bool {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap_or_else(|e| {
            panic!("Failed to compile regex: {e}");
        })
    });
    RE.is_match(name)
}

#[derive(Clone)]
pub struct LocalSession {
    env: HashMap<String, String>,
    pub elevation: Elevation,
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
    changed: Option<bool>,
}

impl LocalSession {
    pub fn new() -> Self {
        Self {
            env: HashMap::new(),
            elevation: Elevation {
                method: ElevationMethod::None,
                as_user: None,
            },
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            exit_code: Some(0),
            changed: Some(false),
        }
    }

    fn execute_command(&self, command: &str) -> Result<(String, String, i32)> {
        let mut full_command = String::new();

        // Set environment variables
        for (key, value) in &self.env {
            if writeln!(full_command, "export {}={}", key, escape_shell_value(value)).is_err() {
                // Writing to a String should not fail, but we handle it just in case
                // to satisfy clippy. In a real-world scenario, this might log an error.
            }
        }

        full_command.push_str(command);

        // Execute via shell
        let output = Command::new("sh")
            .arg("-c")
            .arg(&full_command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok((stdout, stderr, exit_code))
    }
}

impl CommandExecutor for LocalSession {
    fn cmd(&mut self, command: &str) -> Result<(String, String, i32)> {
        let (stdout, stderr, exit_code) = self.execute_command(command)?;

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
        self.execute_command(command)
    }

    fn prepare_command(&self, command: &str) -> String {
        match self.elevation.method {
            ElevationMethod::Su => {
                let escaped_command = escape_shell_value(command);
                self.elevation.as_user.as_ref().map_or_else(
                    || format!("su -c {escaped_command}"),
                    |user| format!("su {user} -c {escaped_command}"),
                )
            }
            ElevationMethod::Sudo => {
                let escaped_command = escape_shell_value(command);
                self.elevation.as_user.as_ref().map_or_else(
                    || format!("sudo -E sh -c {escaped_command}"),
                    |user| format!("sudo -E -u {user} sh -c {escaped_command}"),
                )
            }
            ElevationMethod::None => command.to_string(),
        }
    }

    fn set_env(&mut self, key: &str, value: &str) {
        *self
            .env
            .entry(key.to_string())
            .or_insert_with(|| value.to_string()) = value.to_string();
    }

    fn get_remote_env(&self, var: &str) -> Result<String> {
        if !is_valid_env_var_name(var) {
            return Err(Error::msg(format!(
                "Invalid environment variable name: {var}"
            )));
        }
        let (stdout, _, _) = self.execute_command(&format!("printenv {var}"))?;
        Ok(stdout)
    }

    fn get_tmpdir(&self) -> Result<String> {
        let (stdout, _, exit_code) = self.execute_command(
            "tmpdir=`for dir in \"$HOME/.komandan/tmp\" \"/tmp/komandan\"; do if [ -d \"$dir\" ] || mkdir -p \"$dir\" 2>/dev/null; then echo \"$dir\"; break; fi; done`; [ -z \"$tmpdir\" ] && { exit 1; } || echo \"$tmpdir\""
        )?;

        if exit_code != 0 {
            return Err(Error::msg("Failed to get temporary directory"));
        }

        Ok(stdout)
    }

    fn upload(&self, local_path: &Path, remote_path: &Path) -> Result<()> {
        // For local execution, upload is just a copy operation
        if local_path.is_dir() {
            copy_dir_all(local_path, remote_path)?;
        } else {
            if let Some(parent) = remote_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(local_path, remote_path)?;
        }
        Ok(())
    }

    fn download(&self, remote_path: &Path, local_path: &Path) -> Result<()> {
        // For local execution, download is just a copy operation
        if remote_path.is_dir() {
            copy_dir_all(remote_path, local_path)?;
        } else {
            if let Some(parent) = local_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(remote_path, local_path)?;
        }
        Ok(())
    }

    fn write_remote_file(&self, remote_path: &Path, content: &[u8]) -> Result<()> {
        if let Some(parent) = remote_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(remote_path)?;
        file.write_all(content)?;
        Ok(())
    }

    fn chmod(&self, remote_path: &Path, mode: &str) -> Result<()> {
        let mode = u32::from_str_radix(mode, 8)
            .map_err(|e| Error::new(e).context(format!("Invalid chmod mode: {mode}")))?;
        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(remote_path, perms)?;
        Ok(())
    }

    fn set_changed(&mut self, changed: bool) {
        self.changed = Some(changed);
    }

    fn get_changed(&self) -> bool {
        self.changed.unwrap_or(false)
    }

    fn get_session_result(&self) -> SessionResult {
        SessionResult {
            stdout: self.stdout.as_ref().unwrap_or(&String::new()).clone(),
            stderr: self.stderr.as_ref().unwrap_or(&String::new()).clone(),
            exit_code: self.exit_code.unwrap_or(-1),
            changed: self.changed.unwrap_or(false),
        }
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

impl UserData for LocalSession {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("cmd", |lua, this, command: String| {
            let command = this.prepare_command(command.as_str());
            let cmd_result = this.cmd(&command);
            let (stdout, stderr, exit_code) = cmd_result?;

            let table = lua.create_table()?;
            table.set("stdout", stdout)?;
            table.set("stderr", stderr)?;
            table.set("exit_code", exit_code)?;

            Ok(table)
        });

        methods.add_method_mut("cmdq", |lua, this, command: String| {
            let command = this.prepare_command(command.as_str());
            let cmd_result = this.cmdq(&command);
            let (stdout, stderr, exit_code) = cmd_result?;

            let table = lua.create_table()?;
            table.set("stdout", stdout)?;
            table.set("stderr", stderr)?;
            table.set("exit_code", exit_code)?;

            Ok(table)
        });

        methods.add_method_mut("requires", |_, this, commands: Value| {
            if !commands.is_table() && !commands.is_string() {
                return Err(RuntimeError(
                    "'requires' must be called with a string or table".to_string(),
                ))
            }

            let commands = if commands.is_string() {
                commands.to_string()?
            } else {
                let commands_table = commands.as_table().ok_or_else(|| RuntimeError("commands is not a table".to_string()))?;
                let mut strings = String::new();
                for i in 1..= commands_table.len()? {
                    let s = commands_table.get::<String>(i)?;
                    strings.push_str(&s);
                    if i < commands_table.len()? {
                        strings.push(' ');
                    }
                }
                strings
            };

            let command = this.prepare_command(format!("cmds=\"{commands}\"; unavailable=\"\"; for cmd in $(echo \"$cmds\"); do command -v \"$cmd\" >/dev/null 2>&1 || unavailable=\"$unavailable, $cmd\"; done; [ -z \"$unavailable\" ] || {{ echo \"${{unavailable#, }}\"; false; }}").as_str());
            let cmd_result = this.cmdq(&command);
            let (stdout, _, exit_code) = cmd_result?;

            if exit_code != 0 {
                return Err(RuntimeError(
                    format!(
                        "required commands not found on the local system: {stdout}"
                    ),
                ))
            }

            Ok(())
        });

        methods.add_method_mut(
            "write_remote_file",
            |_, this, (remote_path, content): (String, String)| {
                this.write_remote_file(Path::new(&remote_path), content.as_bytes())?;
                Ok(())
            },
        );

        methods.add_method_mut(
            "upload",
            |_, this, (local_path, remote_path): (String, String)| {
                this.upload(
                    Path::new(local_path.as_str()),
                    Path::new(remote_path.as_str()),
                )?;
                Ok(())
            },
        );

        methods.add_method_mut(
            "download",
            |_, this, (remote_path, local_path): (String, String)| {
                this.download(
                    Path::new(remote_path.as_str()),
                    Path::new(local_path.as_str()),
                )?;
                Ok(())
            },
        );

        methods.add_method_mut("get_remote_env", |_, this, var: String| {
            let val = this.get_remote_env(&var)?;
            Ok(val)
        });

        methods.add_method_mut("get_tmpdir", |_, this, ()| {
            let tmpdir = this.get_tmpdir()?;
            Ok(tmpdir)
        });

        methods.add_method_mut("chmod", |_, this, (remote_path, mode): (String, String)| {
            this.chmod(Path::new(remote_path.as_str()), mode.as_str())?;
            Ok(())
        });

        methods.add_method_mut("set_changed", |_, this, changed: bool| {
            this.set_changed(changed);
            Ok(())
        });

        methods.add_method_mut("get_changed", |_, this, ()| Ok(this.get_changed()));

        methods.add_method("get_session_result", |lua, this, ()| {
            let result = this.get_session_result();
            let table = lua.create_table()?;
            table.set("stdout", result.stdout)?;
            table.set("stderr", result.stderr)?;
            table.set("exit_code", result.exit_code)?;
            table.set("changed", result.changed)?;
            Ok(table)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_session_new() {
        let session = LocalSession::new();
        assert_eq!(session.elevation.method, ElevationMethod::None);
        assert!(session.env.is_empty());
    }

    #[test]
    fn test_set_env() {
        let mut session = LocalSession::new();
        session.set_env("TEST_KEY", "TEST_VALUE");
        assert_eq!(session.env.get("TEST_KEY"), Some(&"TEST_VALUE".to_string()));
    }

    #[test]
    fn test_prepare_command() {
        let mut session = LocalSession::new();

        // Test without elevation
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "ls -la");

        // Test with sudo elevation
        session.elevation.method = ElevationMethod::Sudo;
        session.elevation.as_user = None;
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "sudo -E sh -c \'ls -la\'");

        // Test with sudo elevation and user
        session.elevation.method = ElevationMethod::Sudo;
        session.elevation.as_user = Some("admin".to_string());
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "sudo -E -u admin sh -c \'ls -la\'");

        // Test with su elevation
        session.elevation.method = ElevationMethod::Su;
        session.elevation.as_user = None;
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "su -c \'ls -la\'");

        // Test with su elevation and user
        session.elevation.method = ElevationMethod::Su;
        session.elevation.as_user = Some("admin".to_string());
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "su admin -c \'ls -la\'");
    }

    #[test]
    fn test_cmd_execution() -> anyhow::Result<()> {
        let mut session = LocalSession::new();
        let (stdout, _stderr, exit_code) = session.cmd("echo 'hello world'")?;
        assert_eq!(stdout, "hello world");
        assert_eq!(exit_code, 0);
        Ok(())
    }
}
