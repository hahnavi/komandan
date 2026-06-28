-- Hosts inventory for this Komandan project.
--
-- This file returns a Lua array (list) of host tables. It is wired into the
-- project via komandan.json -> "defaults" -> "hosts": "hosts.lua", and loaded
-- at runtime by calling komandan.defaults:get_hosts() from your task script
-- (see main.lua).
--
-- Each entry is a single host. Only `address` is required; every other field
-- is optional. Available fields (see src/models.rs `Host` for the source of
-- truth):
--
--   address            (string, required)  Hostname / IP. "localhost",
--                                          "127.0.0.1", "::1" imply a local
--                                          (non-SSH) connection.
--   name               (string, optional)  Friendly label for reports/logs.
--   port               (number, optional)  SSH port (default 22). Local only.
--   user               (string, optional)  Login user. Defaults to current user
--                                          for SSH.
--   host_key_check     (bool,   optional)  Verify the SSH host key (default true).
--   private_key_file   (string, optional)  Path to an SSH private key.
--   private_key_pass   (string, optional)  Passphrase for the private key.
--   password           (string, optional)  SSH password auth (avoid if possible).
--   elevate            (bool,   optional)  Run modules with privilege elevation.
--   elevation_method   (string, optional)  "sudo" | "su" | "none" (default "sudo").
--   as_user            (string, optional)  Target user for elevation
--                                          (e.g. "root"). Requires elevate=true.
--   env                (table,  optional)  Extra environment variables
--                                          (string -> string) for module runs.
--   connection         (string, optional)  Force "ssh" or "local"; otherwise
--                                          inferred from address.
return {
	-- Active entry: runs out of the box over the local connection.
	{
		address = "localhost",
	},

	-- Example remote host. Uncomment and edit to suit your environment.
	-- {
	-- 	name = "web-1",
	-- 	address = "192.168.1.10",
	-- 	port = 22,
	-- 	user = "deploy",
	-- 	host_key_check = true,
	-- 	private_key_file = "~/.ssh/id_ed25519",
	-- 	-- private_key_pass = "",   -- passphrase, if your key is encrypted
	-- 	elevate = true,
	-- 	elevation_method = "sudo", -- "sudo" | "su" | "none"
	-- 	as_user = "root",
	-- 	env = {
	-- 		DEPLOY_ENV = "production",
	-- 	},
	-- 	connection = "ssh",        -- "ssh" | "local" (usually inferred)
	-- },
}
