local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_path = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Create a directory",
	komandan.modules.cmd({
		cmd = "mkdir /tmp/newdir",
	}),
}

komandan.komando(host, task)
