local host = {
    name = "My Server",
    address = "localhost",
    user = "usertest",
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
        name = "Task 2",
        komandan.modules.cmd({
            cmd = "ping google.com -c 7",
        }),
    },
    {
        name = "Task 3",
        komandan.modules.apt({
            package = "neovim",
            update_cache = true
        }),
        elevate = true,
    }
}

komandan.komando_parallel_tasks(host, tasks)
