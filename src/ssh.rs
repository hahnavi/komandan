use std::{
    io::{Read, Write},
    net::TcpStream,
    path::Path,
};

use anyhow::Result;
use mlua::{Lua, Table, UserData};
use ssh2::Session;

pub struct SSHSession {
    session: Session,
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
}

impl SSHSession {
    pub fn connect(lua: &Lua, host: &Table) -> Result<Self> {
        let tcp = TcpStream::connect((
            host.get::<String>("address").unwrap().as_str(),
            host.get::<u16>("port").unwrap_or(22),
        ))
        .unwrap();

        let mut session = Session::new().unwrap();

        session.set_tcp_stream(tcp);
        session.handshake().unwrap();

        let defaults = lua
            .globals()
            .get::<Table>("komandan")?
            .get::<Table>("defaults")?;

        let username = match host.get::<String>("user") {
            Ok(user) => user,
            Err(_) => match defaults.get::<String>("user") {
                Ok(user) => user,
                Err(_) => return Err(anyhow::Error::msg("No user specified.")),
            },
        };

        session
            .userauth_pubkey_file(
                &username,
                None,
                Path::new(host.get::<String>("private_key_path").unwrap().as_str()),
                None,
            )
            .unwrap();

        if !session.authenticated() {
            return Err(anyhow::Error::msg("SSH authentication failed."));
        }

        Ok(Self {
            session,
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
        channel.wait_close().unwrap();
        let exit_code = channel.exit_status().unwrap();

        self.stdout.as_mut().unwrap().push_str(&stdout);
        self.stderr.as_mut().unwrap().push_str(&stderr);
        self.exit_code = Some(exit_code);

        Ok((stdout, stderr, exit_code))
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

impl UserData for SSHSession {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("cmd", |lua, this, command: String| {
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

        methods.add_method("get_session_results", |lua, this, ()| {
            let table = lua.create_table()?;
            table.set("stdout", this.stdout.as_ref().unwrap().clone())?;
            table.set("stderr", this.stderr.as_ref().unwrap().clone())?;
            table.set("exit_code", this.exit_code.unwrap())?;
            Ok(table)
        });
    }
}
