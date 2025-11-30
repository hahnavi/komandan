use anyhow::{Result, anyhow};
use komandan::executor::CommandExecutor;
use komandan::ssh::{SSHAuthMethod, SSHSession};
use mlua::Lua;
use std::fs;
use std::io::Write;
use tempfile::{NamedTempFile, TempDir};

fn create_ssh_session() -> Result<SSHSession> {
    let mut session = SSHSession::new()?;
    let home = std::env::var("HOME")?;
    let private_key_path = format!("{home}/.ssh/id_ed25519");

    session.connect(
        "localhost",
        22,
        "usertest",
        SSHAuthMethod::PublicKey {
            private_key: private_key_path,
            passphrase: None,
        },
    )?;

    Ok(session)
}

#[test]
fn test_lua_cmd_execution() -> Result<()> {
    let session = create_ssh_session()?;
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("session", session)?;

    // Test cmd
    lua.load(
        r#"
        local result = session:cmd("echo hello")
        assert(result.stdout == "hello")
        assert(result.exit_code == 0)
    "#,
    )
    .exec()?;

    // Test cmdq
    lua.load(
        r#"
        local result = session:cmdq("echo quiet")
        assert(result.stdout == "quiet")
        assert(result.exit_code == 0)
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_lua_requires() -> Result<()> {
    let session = create_ssh_session()?;
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("session", session)?;

    // Test string input
    lua.load(
        r#"
        session:requires("ls")
    "#,
    )
    .exec()?;

    // Test table input
    lua.load(
        r#"
        session:requires({"ls", "echo"})
    "#,
    )
    .exec()?;

    // Test failure
    let result = lua
        .load(
            r#"
        session:requires("nonexistent_command")
    "#,
        )
        .exec();
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_lua_file_operations() -> Result<()> {
    let session = create_ssh_session()?;
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("session", session.clone())?;

    // Get tmpdir via Lua
    let tmpdir: String = lua
        .load(
            r"
        return session:get_tmpdir()
    ",
        )
        .eval()?;

    // Test write_remote_file
    let remote_file = format!("{tmpdir}/lua_test_file");
    globals.set("remote_file", remote_file.clone())?;
    lua.load(
        r#"
        session:write_remote_file(remote_file, "lua content")
    "#,
    )
    .exec()?;

    // Verify content via cmd
    let (stdout, _, _) = session.cmdq(&format!("cat {remote_file}"))?;
    assert_eq!(stdout, "lua content");

    // Test chmod
    lua.load(
        r#"
        session:chmod(remote_file, "700")
    "#,
    )
    .exec()?;

    let (stdout, _, _) = session.cmdq(&format!("stat -c %a {remote_file}"))?;
    assert_eq!(stdout, "700");

    // Test upload
    let mut local_file = NamedTempFile::new()?;
    local_file.write_all(b"upload content")?;
    let local_path = local_file
        .path()
        .to_str()
        .ok_or_else(|| anyhow!("Failed to convert path to string"))?
        .to_string();
    let remote_upload_path = format!("{tmpdir}/lua_upload");

    globals.set("local_path", local_path)?;
    globals.set("remote_upload_path", remote_upload_path.clone())?;

    lua.load(
        r"
        session:upload(local_path, remote_upload_path)
    ",
    )
    .exec()?;

    let (stdout, _, _) = session.cmdq(&format!("cat {remote_upload_path}"))?;
    assert_eq!(stdout, "upload content");

    // Test download
    let download_dir = TempDir::new()?;
    let local_download_path = download_dir
        .path()
        .join("lua_download")
        .to_str()
        .ok_or_else(|| anyhow!("Failed to convert path to string"))?
        .to_string();
    globals.set("local_download_path", local_download_path.clone())?;

    lua.load(
        r"
        session:download(remote_upload_path, local_download_path)
    ",
    )
    .exec()?;

    let content = fs::read_to_string(local_download_path)?;
    assert_eq!(content, "upload content");

    Ok(())
}

#[test]
fn test_lua_env_and_tmpdir() -> Result<()> {
    let session = create_ssh_session()?;
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("session", session)?;

    // Test get_remote_env
    lua.load(
        r#"
        local user = session:get_remote_env("USER")
        assert(user == "usertest")
    "#,
    )
    .exec()?;

    // Test get_tmpdir
    lua.load(
        r#"
        local tmpdir = session:get_tmpdir()
        assert(string.find(tmpdir, "komandan"))
    "#,
    )
    .exec()?;

    Ok(())
}

#[test]
fn test_lua_state_management() -> Result<()> {
    let session = create_ssh_session()?;
    let lua = Lua::new();
    let globals = lua.globals();
    globals.set("session", session)?;

    // Test set_changed and get_changed
    lua.load(
        r"
        assert(session:get_changed() == false)
        session:set_changed(true)
        assert(session:get_changed() == true)
    ",
    )
    .exec()?;

    // Test get_session_result
    lua.load(
        r#"
        session:cmd("echo result_test")
        local result = session:get_session_result()
        assert(result.stdout == "result_test")
        assert(result.exit_code == 0)
    "#,
    )
    .exec()?;

    Ok(())
}
