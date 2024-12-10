return {
	{
		name = "server1",
		address = "10.20.30.41",
		user = "user2",
		tags = { "webserver" },
	},
	{
		name = "server2",
		address = "10.20.30.42",
		user = "user2",
		tags = { "dbserver" },
	},
	{
		address = "10.20.30.43",
		private_key_path = os.getenv("HOME") .. "/.ssh/id_ed25519",
		tags = { "dbserver" },
	},
}
