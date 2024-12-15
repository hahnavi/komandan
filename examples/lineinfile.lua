local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Add a line in a file",
	komandan.modules.lineinfile({
		path = "/tmp/target_file.txt",
        line = "This is a new line",
        state = "present",
        create = true,
        backup = true,
	}),
}

komandan.komando(host, task)
