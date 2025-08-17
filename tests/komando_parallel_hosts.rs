use komandan::create_lua;
use mlua::{Integer, Table, Value, chunk};

#[test]
fn test_komando_parallel_hosts() {
    let lua = create_lua().unwrap();

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

            return komandan.komando_parallel_hosts(hosts, task)
        })
        .eval::<Table>()
        .unwrap();

    for pair in results.pairs::<Value, Table>() {
        let (_, table) = pair.unwrap();
        assert!(table.get::<Integer>("exit_code").unwrap() == 0);
    }
}
