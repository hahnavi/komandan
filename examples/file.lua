local host = {
	name = "server1",
	address = "10.180.230.174",
	user = "ubuntu",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Remove a file",
	komandan.modules.file({
		path = "/tmp/test.txt",
        state = "file",
		mode = 440,
		owner = "ubuntu",
		group = "group1",
	}),
    elevate = true,
}

komandan.komando(host, task)
