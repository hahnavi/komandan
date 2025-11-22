use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    net::TcpStream,
    path::Path,
};

use anyhow::{Error, Result};
use mlua::{Error::RuntimeError, UserData, Value};
use ssh2::{CheckResult, KnownHostFileKind, Session, Sftp};

#[derive(Debug, PartialEq, Eq)]
pub enum SSHAuthMethod {
    Password(String),
    PublicKey {
        private_key: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Elevation {
    pub method: ElevationMethod,
    pub as_user: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElevationMethod {
    None,
    Su,
    Sudo,
}

#[derive(Clone)]
pub struct SSHSession {
    pub session: Session,
    pub known_hosts_file: Option<String>,
    env: HashMap<String, String>,
    pub elevation: Elevation,
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
    changed: Option<bool>,
}

impl SSHSession {
    pub fn new() -> Result<Self> {
        Ok(Self {
            session: Session::new()?,
            known_hosts_file: None,
            env: HashMap::new(),
            elevation: Elevation {
                method: ElevationMethod::None,
                as_user: None,
            },
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            exit_code: Some(0),
            changed: Some(false),
        })
    }

    pub fn connect(
        &mut self,
        address: &str,
        port: u16,
        username: &str,
        auth_method: SSHAuthMethod,
    ) -> Result<()> {
        let tcp = TcpStream::connect((address, port))?;

        self.session.set_tcp_stream(tcp);
        self.session.handshake()?;

        if let Some(file) = &self.known_hosts_file {
            let host_key = self
                .session
                .host_key()
                .ok_or_else(|| anyhow::anyhow!("Host key is None"))?;
            let mut known_hosts = self.session.known_hosts()?;
            match known_hosts.read_file(Path::new(file.as_str()), KnownHostFileKind::OpenSSH) {
                Ok(_) => {}
                Err(_) => {
                    return Err(Error::msg(format!(
                        "SSH host key verification failed. Please add the host key to the known_hosts file: {file}"
                    )));
                }
            }

            let known_hosts_check_result = known_hosts.check(address, host_key.0);
            match known_hosts_check_result {
                CheckResult::Match => {}
                _ => {
                    return Err(Error::msg(format!(
                        "SSH host key verification failed ({known_hosts_check_result:?}). Please check the known_hosts file: {file}"
                    )));
                }
            }
        }

        match auth_method {
            SSHAuthMethod::Password(password) => {
                self.session.userauth_password(username, &password)?;
            }
            SSHAuthMethod::PublicKey {
                private_key,
                passphrase,
            } => {
                self.session.userauth_pubkey_file(
                    username,
                    None,
                    Path::new(&private_key),
                    passphrase.as_deref(),
                )?;
            }
        }

        if !self.session.authenticated() {
            return Err(Error::msg("SSH authentication failed."));
        }

        Ok(())
    }

    pub fn set_env(&mut self, key: &str, value: &str) {
        *self
            .env
            .entry(key.to_string())
            .or_insert_with(|| value.to_string()) = value.to_string();
    }

    fn execute_command(&self, command: &str) -> Result<ssh2::Channel> {
        let mut channel = self.session.channel_session()?;
        let mut command = command.to_string();
        for (key, value) in &self.env {
            command = format!("export {key}={value}\n") + &command;
        }
        channel.exec(command.as_str())?;
        Ok(channel)
    }

    pub fn cmd(&mut self, command: &str) -> Result<(String, String, i32)> {
        let mut channel = self.execute_command(command)?;
        let mut stdout = String::new();
        let mut stderr = String::new();

        channel.read_to_string(&mut stdout)?;
        channel.stderr().read_to_string(&mut stderr)?;
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close()?;
        let exit_code = channel.exit_status()?;

        if let Some(stdout_buf) = self.stdout.as_mut() {
            stdout_buf.push_str(&stdout);
        }
        if let Some(stderr_buf) = self.stderr.as_mut() {
            stderr_buf.push_str(&stderr);
        }
        self.exit_code = Some(exit_code);

        Ok((stdout, stderr, exit_code))
    }

    pub fn cmdq(&self, command: &str) -> Result<(String, String, i32)> {
        let mut channel = self.execute_command(command)?;
        let mut stdout = String::new();
        let mut stderr = String::new();

        channel.read_to_string(&mut stdout)?;
        channel.stderr().read_to_string(&mut stderr)?;
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close()?;
        let exit_code = channel.exit_status()?;

        Ok((stdout, stderr, exit_code))
    }

    pub fn prepare_command(&self, command: &str) -> String {
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

    pub fn get_remote_env(&self, var: &str) -> Result<String> {
        let mut channel = self.execute_command(format!("echo ${var}").as_str())?;
        let mut stdout = String::new();
        channel.read_to_string(&mut stdout)?;
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close()?;

        Ok(stdout)
    }

    pub fn get_tmpdir(&self) -> Result<String> {
        let mut channel = self.execute_command("tmpdir=`for dir in \"$HOME/.komandan/tmp\" \"/tmp/komandan\"; do if [ -d \"$dir\" ] || mkdir -p \"$dir\" 2>/dev/null; then echo \"$dir\"; break; fi; done`; [ -z \"$tmpdir\" ] && { exit 1; } || echo \"$tmpdir\"")?;
        let mut stdout = String::new();
        channel.read_to_string(&mut stdout)?;
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close()?;

        Ok(stdout)
    }

    pub fn chmod(&self, remote_path: &Path, mode: &String) -> Result<()> {
        self.execute_command(format!("chmod {} {}", mode, remote_path.to_string_lossy()).as_str())?;

        Ok(())
    }

    pub fn upload(&self, local_path: &Path, remote_path: &Path) -> Result<()> {
        let sftp = self.session.sftp()?;

        if local_path.is_dir() {
            upload_directory(&sftp, local_path, remote_path)?;
        } else {
            upload_file(&sftp, local_path, remote_path)?;
        }

        Ok(())
    }

    pub fn download(&self, remote_path: &Path, local_path: &Path) -> Result<()> {
        let sftp = self.session.sftp()?;

        if remote_path.is_dir() {
            download_directory(&sftp, remote_path, local_path)?;
        } else {
            download_file(&sftp, remote_path, local_path)?;
        }

        Ok(())
    }

    pub fn write_remote_file(&self, remote_path: &str, content: &[u8]) -> Result<()> {
        let content_length = content.len() as u64;
        let mut remote_file =
            self.session
                .scp_send(Path::new(remote_path), 0o644, content_length, None)?;
        remote_file.write_all(content)?;
        remote_file.send_eof()?;
        remote_file.wait_eof()?;
        remote_file.close()?;
        remote_file.wait_close()?;

        Ok(())
    }
}

fn upload_file(sftp: &Sftp, local_path: &Path, remote_path: &Path) -> io::Result<()> {
    let mut local_file = fs::File::open(local_path)?;
    let mut remote_file = sftp.create(remote_path)?;

    io::copy(&mut local_file, &mut remote_file)?;

    Ok(())
}

fn upload_directory(sftp: &Sftp, local_path: &Path, remote_path: &Path) -> io::Result<()> {
    if sftp.stat(remote_path).is_err() {
        sftp.mkdir(remote_path, 0o755)?;
    }

    for entry in fs::read_dir(local_path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let entry_name = entry.file_name();
        let remote_entry_path = remote_path.join(entry_name);

        if entry_path.is_dir() {
            upload_directory(sftp, &entry_path, &remote_entry_path)?;
        } else {
            upload_file(sftp, &entry_path, &remote_entry_path)?;
        }
    }

    Ok(())
}

fn download_file(sftp: &Sftp, remote_path: &Path, local_path: &Path) -> io::Result<()> {
    let mut remote_file = sftp.open(remote_path)?;
    let mut local_file = fs::File::create(local_path)?;

    io::copy(&mut remote_file, &mut local_file)?;

    Ok(())
}

fn download_directory(sftp: &Sftp, remote_path: &Path, local_path: &Path) -> io::Result<()> {
    if !local_path.exists() {
        fs::create_dir_all(local_path)?;
    }

    for entry in sftp.readdir(remote_path)? {
        let entry_name = entry
            .0
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid filename"))?;
        let remote_entry_path = remote_path.join(entry_name);
        let local_entry_path = local_path.join(entry_name);

        if entry_name == "." || entry_name == ".." {
            continue;
        }

        if entry.1.file_type() == ssh2::FileType::Directory {
            download_directory(sftp, &remote_entry_path, &local_entry_path)?;
        } else {
            download_file(sftp, &remote_entry_path, &local_entry_path)?;
        }
    }

    Ok(())
}

impl UserData for SSHSession {
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
                        "required commands not found on the remote host: {stdout}"
                    ),
                ))
            }

            Ok(())
        });

        methods.add_method_mut(
            "write_remote_file",
            |_, this, (remote_path, content): (String, String)| {
                this.write_remote_file(remote_path.as_str(), content.as_bytes())?;
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
            this.chmod(Path::new(remote_path.as_str()), &mode)?;
            Ok(())
        });

        methods.add_method_mut("set_changed", |_, this, changed: bool| {
            this.changed = Some(changed);
            Ok(())
        });

        methods.add_method_mut("get_changed", |_, this, ()| {
            Ok(this.changed.unwrap_or(false))
        });

        methods.add_method("get_session_result", |lua, this, ()| {
            let table = lua.create_table()?;
            table.set(
                "stdout",
                this.stdout
                    .as_ref()
                    .ok_or_else(|| RuntimeError("stdout is None".to_string()))?
                    .clone(),
            )?;
            table.set(
                "stderr",
                this.stderr
                    .as_ref()
                    .ok_or_else(|| RuntimeError("stderr is None".to_string()))?
                    .clone(),
            )?;
            table.set(
                "exit_code",
                this.exit_code
                    .ok_or_else(|| RuntimeError("exit_code is None".to_string()))?,
            )?;
            table.set(
                "changed",
                this.changed
                    .ok_or_else(|| RuntimeError("changed is None".to_string()))?,
            )?;
            Ok(table)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_session_new() {
        let session = SSHSession::new();
        assert!(session.is_ok());
        if let Ok(session) = session {
            assert_eq!(session.elevation.method, ElevationMethod::None);
            assert!(session.env.is_empty());
        }
    }

    #[test]
    fn test_set_env() -> anyhow::Result<()> {
        let mut session = SSHSession::new()?;
        session.set_env("TEST_KEY", "TEST_VALUE");
        assert_eq!(session.env.get("TEST_KEY"), Some(&"TEST_VALUE".to_string()));
        Ok(())
    }

    #[test]
    fn test_prepare_command() -> anyhow::Result<()> {
        let mut session = SSHSession::new()?;

        // Test without elevation
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "ls -la");

        // Test with sudo elevation
        session.elevation.method = ElevationMethod::Sudo;
        session.elevation.as_user = None;
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "sudo -E ls -la");

        // Test with sudo elevation and user
        session.elevation.method = ElevationMethod::Sudo;
        session.elevation.as_user = Some("admin".to_string());
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "sudo -E -u admin ls -la");

        // Test with su elevation
        session.elevation.method = ElevationMethod::Su;
        session.elevation.as_user = None;
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "su -c 'ls -la'");

        // Test with su elevation and user
        session.elevation.method = ElevationMethod::Su;
        session.elevation.as_user = Some("admin".to_string());
        let cmd = session.prepare_command("ls -la");
        assert_eq!(cmd, "su admin -c 'ls -la'");
        Ok(())
    }
}
