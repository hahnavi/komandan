local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Install neovim",
	komandan.modules.apt({
		package = "neovim",
        update_cache = true
	}),
    elevate = true,
}

komandan.komando(host, task)
