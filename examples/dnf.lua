local host = {
	name = "server1",
	address = "192.168.98.26",
	user = "cloud-user",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Install vim",
	komandan.modules.dnf({
		package = "vim",
	}),
    elevate = true,
}

komandan.komando(host, task)
