local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Write a file using a template",
	komandan.modules.template({
		src = "/tmp/template1.j2",
        dst = "/tmp/target_file.txt",
        vars = {
            name = "John Doe",
            age = 30
        }
	}),
}

komandan.komando(host, task)


-- Jinja template example:
-- {{ name }} is {{ age }} years old
