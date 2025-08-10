use komandan::create_lua;
use mlua::{Integer, Table, Value, chunk};

#[test]
fn test_komando_parallel_tasks() {
    let lua = create_lua().unwrap();

    let results = lua
        .load(chunk! {
            local host = {
                name = "My Server",
                address = "localhost",
                port = 2222,
                user = "usertest",
                host_key_check = false,
                private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
            }

            local tasks = {
                {
                    name = "Task 1",
                    komandan.modules.cmd({
                        cmd = "uname -a",
                    }),
                },
                {
                    name = "Echo 1",
                    komandan.modules.cmd({
                        cmd = "echo 1",
                    }),
                },
                {
                    name = "Echo 2",
                    komandan.modules.cmd({
                        cmd = "echo 2"
                    }),
                }
            }

            return komandan.komando_parallel_tasks(host, tasks)
        })
        .eval::<Table>()
        .unwrap();

    for pair in results.pairs::<Value, Table>() {
        let (_, table) = pair.unwrap();
        assert!(table.get::<Integer>("exit_code").unwrap() == 0);
    }
}
