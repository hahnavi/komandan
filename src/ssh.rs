use std::{
    fs,
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::Path,
};

use anyhow::Result;
use mlua::UserData;
use ssh2::{Session, Sftp};

pub enum SSHAuthMethod {
    Password(String),
    PublicKey {
        private_key: String,
        passphrase: Option<String>,
    },
}

pub struct Elevation {
    pub method: ElevateMethod,
    pub as_user: Option<String>,
}

pub enum ElevateMethod {
    None,
    Su,
    Sudo,
}

pub struct SSHSession {
    session: Session,
    elevation: Elevation,
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
}

impl SSHSession {
    pub fn connect<A: ToSocketAddrs>(
        addr: A,
        username: &str,
        auth_method: SSHAuthMethod,
        elevation: Elevation,
    ) -> Result<Self> {
        let tcp = TcpStream::connect(addr)?;
        let mut session = Session::new()?;

        session.set_tcp_stream(tcp);
        session.handshake()?;

        match auth_method {
            SSHAuthMethod::Password(password) => {
                session.userauth_password(&username, &password)?;
            }
            SSHAuthMethod::PublicKey {
                private_key,
                passphrase,
            } => {
                session.userauth_pubkey_file(
                    &username,
                    None,
                    Path::new(&private_key),
                    passphrase.as_deref(),
                )?;
            }
        }

        if !session.authenticated() {
            return Err(anyhow::Error::msg("SSH authentication failed."));
        }

        Ok(Self {
            session,
            elevation,
            stdout: Some(String::new()),
            stderr: Some(String::new()),
            exit_code: Some(0),
        })
    }

    pub fn cmd(&mut self, command: &str) -> Result<(String, String, i32)> {
        let mut channel = self.session.channel_session().unwrap();
        channel.exec(command).unwrap();
        let mut stdout = String::new();
        let mut stderr = String::new();

        channel.read_to_string(&mut stdout).unwrap();
        channel.stderr().read_to_string(&mut stderr).unwrap();
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close().unwrap();
        let exit_code = channel.exit_status().unwrap();

        self.stdout.as_mut().unwrap().push_str(&stdout);
        self.stderr.as_mut().unwrap().push_str(&stderr);
        self.exit_code = Some(exit_code);

        Ok((stdout, stderr, exit_code))
    }

    pub fn prepare_command(&mut self, command: &str) -> Result<String> {
        let command = match self.elevation.method {
            ElevateMethod::Su => match &self.elevation.as_user {
                Some(user) => format!("su {} -c '{}'", user, command),
                None => format!("su -c '{}'", command),
            },
            ElevateMethod::Sudo => match &self.elevation.as_user {
                Some(user) => format!("sudo -u {} {}", user, command),
                None => format!("sudo {}", command),
            },
            _ => command.to_string(),
        };

        Ok(command)
    }

    pub fn get_remote_env(&mut self, var: &str) -> Result<String> {
        let mut channel = self.session.channel_session().unwrap();
        channel.exec(format!("echo ${}", var).as_str()).unwrap();
        let mut stdout = String::new();
        channel.read_to_string(&mut stdout).unwrap();
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close().unwrap();

        Ok(stdout)
    }

    pub fn get_tmpdir(&mut self) -> Result<String> {
        let mut channel = self.session.channel_session().unwrap();
        channel.exec("tmpdir=`for dir in \"$HOME/.komandan/tmp\" \"/tmp/komandan\"; do if [ -d \"$dir\" ] || mkdir -p \"$dir\" 2>/dev/null; then echo \"$dir\"; break; fi; done`; [ -z \"$tmpdir\" ] && { exit 1; } || echo \"$tmpdir\"").unwrap();
        let mut stdout = String::new();
        channel.read_to_string(&mut stdout).unwrap();
        stdout = stdout.trim_end_matches('\n').to_string();
        channel.wait_close().unwrap();

        Ok(stdout)
    }

    pub fn chmod(&mut self, remote_path: &Path, mode: &String) -> Result<()> {
        let mut channel = self.session.channel_session().unwrap();
        channel
            .exec(format!("chmod {} {}", mode, remote_path.to_string_lossy()).as_str())
            .unwrap();

        Ok(())
    }

    pub fn upload(&mut self, local_path: &Path, remote_path: &Path) -> Result<()> {
        let mut sftp = self.session.sftp()?;

        if local_path.is_dir() {
            upload_directory(&mut sftp, local_path, remote_path)?;
        } else {
            upload_file(&mut sftp, local_path, remote_path)?;
        }

        Ok(())
    }

    pub fn download(&mut self, remote_path: &Path, local_path: &Path) -> Result<()> {
        let mut sftp = self.session.sftp()?;

        if remote_path.is_dir() {
            download_directory(&mut sftp, remote_path, local_path)?;
        } else {
            download_file(&mut sftp, remote_path, local_path)?;
        }

        Ok(())
    }

    pub fn write_remote_file(&mut self, remote_path: &str, content: &[u8]) -> Result<()> {
        let content_length = content.len() as u64;
        let mut remote_file = self
            .session
            .scp_send(Path::new(remote_path), 0o644, content_length, None)
            .unwrap();
        remote_file.write(content).unwrap();
        remote_file.send_eof().unwrap();
        remote_file.wait_eof().unwrap();
        remote_file.close().unwrap();
        remote_file.wait_close().unwrap();

        Ok(())
    }
}

fn upload_file(sftp: &mut Sftp, local_path: &Path, remote_path: &Path) -> io::Result<()> {
    let mut local_file = fs::File::open(local_path)?;
    let mut remote_file = sftp.create(remote_path)?;

    io::copy(&mut local_file, &mut remote_file)?;

    Ok(())
}

fn upload_directory(sftp: &mut Sftp, local_path: &Path, remote_path: &Path) -> io::Result<()> {
    if !sftp.stat(remote_path).is_ok() {
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

fn download_file(sftp: &mut Sftp, remote_path: &Path, local_path: &Path) -> io::Result<()> {
    let mut remote_file = sftp.open(remote_path)?;
    let mut local_file = fs::File::create(local_path)?;

    io::copy(&mut remote_file, &mut local_file)?;

    Ok(())
}

fn download_directory(sftp: &mut Sftp, remote_path: &Path, local_path: &Path) -> io::Result<()> {
    if !local_path.exists() {
        fs::create_dir_all(local_path)?;
    }

    for entry in sftp.readdir(remote_path)? {
        let entry_name = entry.0.file_name().unwrap();
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
            let command = this.prepare_command(command.as_str()).unwrap();
            let cmd_result = this.cmd(&command);
            let (stdout, stderr, exit_code) = cmd_result.unwrap();

            let table = lua.create_table()?;
            table.set("stdout", stdout)?;
            table.set("stderr", stderr)?;
            table.set("exit_code", exit_code)?;

            Ok(table)
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
            let tmpdir = this.get_tmpdir().unwrap();
            Ok(tmpdir)
        });

        methods.add_method_mut("chmod", |_, this, (remote_path, mode): (String, String)| {
            this.chmod(Path::new(remote_path.as_str()), &mode)?;
            Ok(())
        });

        methods.add_method("get_session_results", |lua, this, ()| {
            let table = lua.create_table()?;
            table.set("stdout", this.stdout.as_ref().unwrap().clone())?;
            table.set("stderr", this.stderr.as_ref().unwrap().clone())?;
            table.set("exit_code", this.exit_code.unwrap())?;
            Ok(table)
        });
    }
}
