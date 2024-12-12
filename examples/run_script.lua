local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task = {
	name = "Run a python script",
	komandan.modules.script({
		script = [[
x = 5
y = 27
print(x * y)
]],
		interpreter = "python3",
	}),
}

local result = komandan.komando(host, task)

print(result.stdout)

