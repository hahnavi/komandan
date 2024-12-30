local hosts = {
    {
        name = "pg1",
        address = "10.180.230.106",
        tags = { "primary" },
    },
    {
        name = "pg2",
        address = "10.180.230.14",
        tags = { "replica" },
    },
}

komandan.defaults:set_user("dbadmin")
komandan.defaults:set_private_key_file(os.getenv("HOME") .. "/.ssh/id_ed25519")
komandan.defaults:set_elevate(true)

local pg_version = "17"

-- Install PostgreSQL on both hosts
local tasks_postgresql_install = {
    {
        name = "Update apt cache",
        komandan.modules.apt({
            update_cache = true,
        }),
    },
    {
        name = "Install postgresql-common",
        komandan.modules.apt({
            package = "postgresql-common",
        }),
    },
    {
        name = "Add PostgreSQL repository",
        env = {
            YES = "yes",
        },
        komandan.modules.cmd({
            cmd = "/usr/share/postgresql-common/pgdg/apt.postgresql.org.sh",
        }),
    },
    {
        name = "Install PostgreSQL " .. pg_version,
        komandan.modules.apt({
            package = "postgresql-" .. pg_version,
        }),
    },
    {
        name = "Update postgresql.conf to listen on all interfaces",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/postgresql.conf",
            line = "listen_addresses = '*'",
        }),
    },
    {
        name = "Update postgresql.conf wal_level",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/postgresql.conf",
            line = "wal_level = replica",
        }),
    },
    {
        name = "Update postgresql.conf max_wal_senders",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/postgresql.conf",
            line = "max_wal_senders = 10",
        }),
    },
    {
        name = "Update postgresql.conf synchronous_commit",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/postgresql.conf",
            line = "synchronous_commit = on",
        }),
    },
    {
        name = "Update pg_hba.conf to add replication",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/pg_hba.conf",
            line = "host    replication     replicator      " .. hosts[1].address .. "/32       scram-sha-256",
        }),
    },
    {
        name = "Update pg_hba.conf to add replication",
        komandan.modules.lineinfile({
            path = "/etc/postgresql/" .. pg_version .. "/main/pg_hba.conf",
            line = "host    replication     replicator      " .. hosts[2].address .. "/32       scram-sha-256",
        }),
    },
    {
        name = "Restart PostgreSQL",
        komandan.modules.systemd_service({
            name = "postgresql",
            state = "restarted",
        }),
    },
}

for _, task in pairs(tasks_postgresql_install) do
    komandan.komando_parallel_hosts(hosts, task)
end

-- Setup PostgreSQL primary
local tasks_setup_primary = {
    {
        name = "Create replication user",
        komandan.modules.cmd({
            cmd = "psql -c \"DO \\$\\$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_user WHERE usename = '" .. os.getenv("REPLICATOR_USER") .. "') THEN CREATE USER " .. os.getenv("REPLICATOR_USER") .. " WITH REPLICATION ENCRYPTED PASSWORD '" .. os.getenv("REPLICATOR_PASSWORD") .. "'; END IF; END \\$\\$;\"",
        }),
        as_user = "postgres",
    },
}

local host_primary = komandan.filter_hosts(hosts, "primary")[1]

for _, task in pairs(tasks_setup_primary) do
    komandan.komando(host_primary, task)
end

-- Setup PostgreSQL replica
local tasks_setup_replica = {
    {
        name = "Stop PostgreSQL",
        komandan.modules.systemd_service({
            name = "postgresql",
            state = "stopped",
        }),
    },
    {
        name = "Delete existing data directory",
        komandan.modules.cmd({
            cmd = "rm -rf /var/lib/postgresql/" .. pg_version .. "/main",
        }),
    },
    {
        name = "Create a new empty data directory",
        komandan.modules.cmd({
            cmd = "mkdir /var/lib/postgresql/" .. pg_version .. "/main",
        }),
        as_user = "postgres",
    },
    {
        name = "Change mode of new data directory",
        komandan.modules.cmd({
            cmd = "chmod 700 /var/lib/postgresql/" .. pg_version .. "/main",
        }),
        as_user = "postgres",
    },
    {
        name = "Synchronize data directory",
        komandan.modules.cmd({
            cmd = "PGPASSWORD='" .. os.getenv("REPLICATOR_PASSWORD") .. "' pg_basebackup -h " .. host_primary.address .. " -U " .. os.getenv("REPLICATOR_USER") .. " -D /var/lib/postgresql/" .. pg_version .. "/main -Fp -Xs -R",
        }),
        as_user = "postgres",
    },
    {
        name = "Start PostgreSQL",
        komandan.modules.systemd_service({
            name = "postgresql",
            state = "started",
        }),
    },
}

local host_replica = komandan.filter_hosts(hosts, "replica")[1]

for _, task in pairs(tasks_setup_replica) do
    komandan.komando(host_replica, task)
end
