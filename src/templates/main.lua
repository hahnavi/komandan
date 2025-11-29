local hosts = komandan.defaults:get_hosts()

local task = {
	name = "Hello world!",
	komandan.modules.cmd({
		cmd = "echo 123",
	}),
}

komandan.komando(hosts[1], task)
