local host = {
	name = "server1",
	address = "ubuntu24",
	user = "ubuntu",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Start a systemd service",
	komandan.modules.systemd_service({
		name = "nginx",
		action = "restart",
	}),
	elevate = true,
}

komandan.komando(host, task)
