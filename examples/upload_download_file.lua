local host = {
	name = "server1",
	address = "10.20.30.41",
	user = "user1",
	private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
}

local task_upload = {
	name = "Upload a file",
	komandan.modules.upload({
		src = "/tmp/local/file1",
		dst = "/tmp/remote/file1",
	}),
}

local task_download = {
	name = "Download a file",
	komandan.modules.download({
		src = "/tmp/remote/file1",
		dst = "/tmp/local/file1_copy",
	}),
}

komandan.komando(host, task_upload)

komandan.komando(host, task_download)
