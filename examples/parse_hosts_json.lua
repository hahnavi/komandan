local hosts = komandan.parse_hosts_json_file("/path/to/hosts.json")
-- or use a URL
-- local hosts = komandan.parse_hosts_json_url("http://localhost:8000/hosts.json")

komandan.defaults:set_user("user1")
komandan.defaults:set_private_key_file(os.getenv("HOME") .. "/.ssh/id_ed25519")

for _, host in pairs(hosts) do
    komandan.komando(host, {
        name = "Create a directory",
        komandan.modules.cmd({
            cmd = "mkdir /tmp/newdir1",
        }),
    })
end
