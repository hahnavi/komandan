use komandan::create_lua;
use mlua::{Integer, Table, Value, chunk};

#[test]
fn test_komando_parallel_hosts() -> mlua::Result<()> {
    if std::env::var("KOMANDAN_SSH_TEST").is_err() {
        eprintln!("Skipping SSH integration test - set KOMANDAN_SSH_TEST=1 to enable");
        return Ok(());
    }
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            local hosts = {
                {
                    name = "server1",
                    address = "localhost",
                    user = "usertest",
                    host_key_check = false,
                    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
                },
                {
                    name = "server2",
                    address = "localhost",
                    user = "usertest",
                    host_key_check = false,
                    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
                },
                {
                    name = "server3",
                    address = "localhost",
                    user = "usertest",
                    host_key_check = false,
                    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
                }
            }

            local task = {
                name = "Echo 1",
                komandan.modules.cmd({
                    cmd = "echo 1",
                }),
            }

            return komandan.komando_parallel_hosts(task, hosts)
        })
        .eval::<Table>()?;

    for pair in results.pairs::<Value, Table>() {
        let (_, table) = pair?;
        assert_eq!(table.get::<Integer>("exit_code")?, 0);
    }
    Ok(())
}
