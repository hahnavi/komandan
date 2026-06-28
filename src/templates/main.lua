-- Main task script for this Komandan project.
--
-- Komandan loads this file (per komandan.json -> "main": "main.lua") and
-- executes it inside a Lua VM that already has the global `komandan` table
-- available. The `komandan` table exposes:
--
--   komandan.defaults:get_hosts()
--       Returns the array of hosts declared in hosts.lua (see that file for
--       field reference). Each element is a Host table you can pass to
--       komandan.komando.
--
--   komandan.modules.<name>({ ... })
--       Builds a module invocation. <name> is one of the built-in modules:
--         apt, cmd, dnf, download, file, get_url, group, lineinfile,
--         postgresql_user, script, systemd_service, template, upload, user
--       Each module takes a single table of options and returns a value ready
--       to embed in a task. See src/modules/<name>.rs for per-module options.
--
--   komandan.komando(task, host)
--       Runs a task against a single host and returns a result table.
--
--   komandan.komando_parallel_tasks(tasks, host)
--       Runs several tasks against one host in parallel.
--
--   komandan.komando_parallel_hosts(task, hosts)
--       Runs one task against many hosts in parallel.
--
-- A task is a Lua table with a string `name` followed by one or more module
-- invocations. The example below runs a single shell command on the first
-- host from hosts.lua.

local hosts = komandan.defaults:get_hosts()

local task = {
	name = "Hello world!",
	komandan.modules.cmd({
		cmd = "echo 123",
	}),
}

komandan.komando(task, hosts[1])
