local hosts = {
    {
        name = "server1",
        address = "localhost",
        user = "usertest",
        private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
    },
    {
        name = "server2",
        address = "localhost",
        user = "usertest",
        private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
    },
    {
        name = "server3",
        address = "localhost",
        user = "usertest",
        private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
    }
}

local task = {
    name = "Ping Google",
    komandan.modules.cmd({
        cmd = "ping google.com -c 5",
    }),
}

komandan.komando_parallel_hosts(hosts, task)
