local host = {
    name = "pg1",
    address = "10.180.230.60",
    tags = { "primary" },
    private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
    user = "ubuntu",
}

local task = {
    name = "Create a user",
    komandan.modules.postgresql_user({
        name = "replicator",
        password = os.getenv("REPLICATOR_PASSWORD"),
        role_attr_flags = "REPLICATION",
    }),
    elevate = true,
    as_user = "postgres",
}

local result = komandan.komando(host, task)

