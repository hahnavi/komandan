local hosts = require("hosts")

komandan.set_defaults({
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
})

local tasks = {
	{
		name = "Create a directory",
		komandan.modules.cmd({
			cmd = "mkdir /tmp/newdir",
		}),
	},
	{
		name = "Delete a directory",
		komandan.modules.cmd({
			cmd = "rm -rf /tmp/newdir",
		}),
	},
}

local filtered_hosts = komandan.filter_hosts(hosts, "dbserver")

for _, task in pairs(tasks) do
	for _, host in pairs(filtered_hosts) do
		komandan.komando(host, task)
	end
end
