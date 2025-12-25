local hosts = komandan.defaults:get_hosts()

local task = {
	name = "Hello world!",
	komandan.modules.cmd({
		cmd = "echo 123",
	}),
}

komandan.komando(task, hosts[1])
