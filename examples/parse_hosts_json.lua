local hosts = komandan.parse_hosts_json("/path/to/hosts.json")

komandan.set_defaults({
    user = "user1",
    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
})

for _, host in pairs(hosts) do
    komandan.komando(host, {
        name = "Create a directory",
        komandan.modules.cmd({
            cmd = "mkdir /tmp/newdir1",
        }),
    })
end
