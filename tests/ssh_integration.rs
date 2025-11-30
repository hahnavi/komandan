use anyhow::Result;
use komandan::executor::CommandExecutor;
use komandan::ssh::{SSHAuthMethod, SSHSession};
use std::fs;
use std::io::Write;
use std::path::Path;
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
fn test_command_execution_with_env_variables() -> Result<()> {
    let mut session = create_ssh_session()?;
    session.set_env("TEST_VAR", "test_value");

    let (stdout, _, exit_code) = session.cmd("echo $TEST_VAR")?;
    assert_eq!(exit_code, 0);
    assert_eq!(stdout, "test_value");

    Ok(())
}

#[test]
fn test_get_remote_env_method() -> Result<()> {
    let session = create_ssh_session()?;
    let result = session.get_remote_env("USER")?;
    assert_eq!(result, "usertest");
    Ok(())
}

#[test]
fn test_get_tmpdir_method() -> Result<()> {
    let session = create_ssh_session()?;
    let result = session.get_tmpdir();
    assert!(result.is_ok());
    let tmpdir = result?;
    assert!(tmpdir.contains("komandan"));
    Ok(())
}

#[test]
fn test_chmod_method() -> Result<()> {
    let session = create_ssh_session()?;
    let tmpdir = session.get_tmpdir()?;
    let remote_path = format!("{tmpdir}/test_chmod");
    let remote_path = Path::new(&remote_path);

    session.write_remote_file(remote_path, b"test content")?;

    session.chmod(remote_path, "755")?;

    let (stdout, _, _) = session.cmdq(&format!("stat -c %a {}", remote_path.display()))?;
    assert_eq!(stdout, "755");

    Ok(())
}

#[test]
fn test_upload_download_methods() -> Result<()> {
    let session = create_ssh_session()?;
    let tmpdir = session.get_tmpdir()?;

    // Create local file
    let mut local_file = NamedTempFile::new()?;
    local_file.write_all(b"upload test content")?;
    let local_path = local_file.path();

    let remote_path_str = format!("{tmpdir}/uploaded_file");
    let remote_path = Path::new(&remote_path_str);

    // Upload
    session.upload(local_path, remote_path)?;

    // Verify upload
    let (stdout, _, _) = session.cmdq(&format!("cat {}", remote_path.display()))?;
    assert_eq!(stdout, "upload test content");

    // Download
    let download_dir = TempDir::new()?;
    let local_download_path = download_dir.path().join("downloaded_file");

    session.download(remote_path, &local_download_path)?;

    // Verify download
    let content = fs::read_to_string(local_download_path)?;
    assert_eq!(content, "upload test content");

    Ok(())
}

#[test]
fn test_write_remote_file_method() -> Result<()> {
    let session = create_ssh_session()?;
    let tmpdir = session.get_tmpdir()?;
    let remote_path_str = format!("{tmpdir}/written_file");
    let remote_path = Path::new(&remote_path_str);

    session.write_remote_file(remote_path, b"written content")?;

    let (stdout, _, _) = session.cmdq(&format!("cat {}", remote_path.display()))?;
    assert_eq!(stdout, "written content");

    Ok(())
}

#[test]
fn test_cmd_method_interface() -> Result<()> {
    let mut session = create_ssh_session()?;
    let (stdout, _, exit_code) = session.cmd("echo test")?;
    assert_eq!(exit_code, 0);
    assert_eq!(stdout, "test");
    Ok(())
}

#[test]
fn test_cmdq_method_interface() -> Result<()> {
    let session = create_ssh_session()?;
    let (stdout, _, exit_code) = session.cmdq("echo test")?;
    assert_eq!(exit_code, 0);
    assert_eq!(stdout, "test");
    Ok(())
}

#[test]
fn test_upload_directory_utility() -> Result<()> {
    let session = create_ssh_session()?;
    let tmpdir = session.get_tmpdir()?;

    // Create local directory structure
    let local_dir = TempDir::new()?;
    let local_path = local_dir.path();
    fs::write(local_path.join("file1.txt"), "content1")?;
    fs::create_dir(local_path.join("subdir"))?;
    fs::write(local_path.join("subdir/file2.txt"), "content2")?;

    let remote_path_str = format!("{tmpdir}/uploaded_dir");
    let remote_path = Path::new(&remote_path_str);

    // Upload directory
    session.upload(local_path, remote_path)?;

    // Verify upload
    let (stdout, _, _) = session.cmdq(&format!("ls {}/file1.txt", remote_path.display()))?;
    assert!(stdout.contains("file1.txt"));

    let (stdout, _, _) = session.cmdq(&format!("ls {}/subdir/file2.txt", remote_path.display()))?;
    assert!(stdout.contains("file2.txt"));

    let (stdout, _, _) = session.cmdq(&format!("cat {}/file1.txt", remote_path.display()))?;
    assert_eq!(stdout, "content1");

    Ok(())
}

#[test]
fn test_download_directory_utility() -> Result<()> {
    let session = create_ssh_session()?;
    let tmpdir = session.get_tmpdir()?;

    // Setup remote directory
    let remote_dir_str = format!("{tmpdir}/download_source_dir");
    let remote_dir = Path::new(&remote_dir_str);
    session.cmdq(&format!("mkdir -p {}/subdir", remote_dir.display()))?;
    session.write_remote_file(&remote_dir.join("file1.txt"), b"content1")?;
    session.write_remote_file(&remote_dir.join("subdir/file2.txt"), b"content2")?;

    // Download directory
    let local_dir = TempDir::new()?;
    let local_path = local_dir.path().join("downloaded_dir");

    session.download(remote_dir, &local_path)?;

    // Verify download
    assert!(local_path.join("file1.txt").exists());
    assert!(local_path.join("subdir/file2.txt").exists());

    let content1 = fs::read_to_string(local_path.join("file1.txt"))?;
    assert_eq!(content1, "content1");

    let content2 = fs::read_to_string(local_path.join("subdir/file2.txt"))?;
    assert_eq!(content2, "content2");

    Ok(())
}
