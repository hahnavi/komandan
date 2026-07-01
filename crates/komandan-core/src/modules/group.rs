use mlua::{ExternalResult, Lua, Table, chunk};

pub fn group(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.name == nil then
                error("'name' parameter is required")
            end

            -- Sanitize group name (allow alphanumeric, underscore, hyphen, dot)
            local function sanitize_group_name(name)
                if type(name) ~= "string" then
                    return nil
                end
                local sanitized = name:gsub("[^%w%-_%.]", "")
                if sanitized == "" then return nil end
                return sanitized
            end

            -- Sanitize numeric values
            local function sanitize_numeric(value)
                if value == nil then return nil end
                if type(value) == "number" then
                    return tostring(math.floor(value))
                elseif type(value) == "string" then
                    local num = value:gsub("[^%d]", "")
                    if num == "" then return nil end
                    return num
                else
                    return nil
                end
            end

            -- Validate and sanitize parameters
            params.name = sanitize_group_name(params.name)
            if params.name == nil then
                error("'name' parameter must be a valid group name")
            end

            local valid_states = {
                present = true,
                absent = true,
            }

            if params.state ~= nil and not valid_states[params.state] then
                error("Invalid state: " .. params.state .. ". Valid states are: present, absent.")
            end

            params.state = params.state or "present"

            -- Sanitize gid if provided
            if params.gid ~= nil then
                params.gid = sanitize_numeric(params.gid)
                if params.gid == nil then
                    error("'gid' parameter must be a valid numeric value")
                end
            end

            -- Note: gid_min and gid_max are not supported on all distributions
            -- These parameters are accepted but ignored for compatibility

            -- Validate boolean parameters
            if params.system ~= nil and type(params.system) ~= "boolean" then
                error("'system' parameter must be a boolean value")
            end

            if params.force ~= nil and type(params.force) ~= "boolean" then
                error("'force' parameter must be a boolean value")
            end

            if params.non_unique ~= nil and type(params.non_unique) ~= "boolean" then
                error("'non_unique' parameter must be a boolean value")
            end

            if params.local_group ~= nil and type(params.local_group) ~= "boolean" then
                error("'local_group' parameter must be a boolean value")
            end

            local module = $base_module:new({ name = "group" })

            module.params = $params

            local function shell_escape(s)
                if s == nil then return "''" end
                return "'" .. string.gsub(s, "'", "'\"'\"'") .. "'"
            end

            local function split(s, delimiter)
                local result = {}
                for match in (s..delimiter):gmatch("(.-)"..delimiter) do
                    table.insert(result, match)
                end
                return result
            end

            module.is_exists = function(self)
                local result = self.ssh:cmdq("getent group " .. shell_escape(self.params.name) .. " >/dev/null 2>&1")
                return result.exit_code == 0
            end

            module.get_group_info = function(self)
                local result = self.ssh:cmdq("getent group " .. shell_escape(self.params.name))
                if result.exit_code ~= 0 then
                    return nil
                end
                local stdout = result.stdout:gsub("%s+$", "")
                local parts = split(stdout, ":")
                return {
                    name = parts[1],
                    password = parts[2],
                    gid = parts[3],
                    members = parts[4] and split(parts[4], ",") or {},
                }
            end

            module.gid_exists = function(self, gid)
                local result = self.ssh:cmdq("getent group " .. shell_escape(gid) .. " >/dev/null 2>&1")
                return result.exit_code == 0
            end

            module.dry_run = function(self)
                local is_exists = self:is_exists()

                if self.params.state == "absent" then
                    if is_exists then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.state == "present" then
                    if not is_exists then
                        self.ssh:set_changed(true)
                    else
                        local current_info = self:get_group_info()

                        -- Check if GID needs to be changed
                        if self.params.gid ~= nil and current_info.gid ~= self.params.gid then
                            self.ssh:set_changed(true)
                        end
                    end
                end
            end

            module.run = function(self)
                local is_exists = self:is_exists()

                if self.params.state == "absent" then
                    if is_exists then
                        local cmd = "groupdel"
                        if self.params.force == true then
                            cmd = cmd .. " --force"
                        end
                        cmd = cmd .. " " .. shell_escape(self.params.name)
                        local result = self.ssh:cmd(cmd)
                        if result.exit_code == 0 then
                            self.ssh:set_changed(true)
                        else
                            error("Failed to delete group: " .. result.stderr)
                        end
                    end
                elseif self.params.state == "present" then
                    if not is_exists then
                        local cmd = "groupadd"

                        -- Add GID if specified
                        if self.params.gid ~= nil then
                            -- Check if GID already exists (unless non_unique is true)
                            if self.params.non_unique ~= true and self:gid_exists(self.params.gid) then
                                error("GID " .. self.params.gid .. " already exists")
                            end
                            cmd = cmd .. " --gid " .. shell_escape(self.params.gid)
                            if self.params.non_unique == true then
                                cmd = cmd .. " --non-unique"
                            end
                        end

                        -- Add system flag if specified
                        if self.params.system == true then
                            cmd = cmd .. " --system"
                        end

                        -- Note: --gid-min and --gid-max are not supported on all distributions
                        -- These options are primarily available on newer versions of shadow-utils
                        -- For compatibility, we skip these options for now

                        -- Add local flag if specified
                        if self.params.local_group == true then
                            cmd = cmd .. " --local"
                        end

                        cmd = cmd .. " " .. shell_escape(self.params.name)
                        local result = self.ssh:cmd(cmd)
                        if result.exit_code == 0 then
                            self.ssh:set_changed(true)
                        else
                            error("Failed to create group: " .. result.stderr)
                        end
                    else
                        -- Group exists, check if we need to modify it
                        local current_info = self:get_group_info()
                        local groupmod_cmd = "groupmod"
                        local groupmod_needed = false

                        -- Check if GID needs to be changed
                        if self.params.gid ~= nil and current_info.gid ~= self.params.gid then
                            -- Check if new GID already exists (unless non_unique is true)
                            if self.params.non_unique ~= true and self:gid_exists(self.params.gid) then
                                error("GID " .. self.params.gid .. " already exists")
                            end
                            groupmod_cmd = groupmod_cmd .. " --gid " .. shell_escape(self.params.gid)
                            if self.params.non_unique == true then
                                groupmod_cmd = groupmod_cmd .. " --non-unique"
                            end
                            groupmod_needed = true
                        end

                        if groupmod_needed then
                            groupmod_cmd = groupmod_cmd .. " " .. shell_escape(self.params.name)
                            local result = self.ssh:cmd(groupmod_cmd)
                            if result.exit_code == 0 then
                                self.ssh:set_changed(true)
                            else
                                error("Failed to modify group: " .. result.stderr)
                            end
                        end
                    end
                end
            end

            return module
            })
        .set_name("group")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

// Tests
#[cfg(test)]
mod tests {
    use crate::create_lua;

    use super::*;

    #[test]
    fn test_group_name_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        let result = group(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("'name' parameter is required"));
        }
        Ok(())
    }

    #[test]
    fn test_group_valid_name() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testgroup")?;
        let result = group(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_group_invalid_state() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testgroup")?;
        params.set("state", "invalid")?;
        let result = group(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Invalid state"));
        }
        Ok(())
    }

    #[test]
    fn test_group_invalid_gid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testgroup")?;
        params.set("gid", "invalid")?;
        let result = group(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("'gid' parameter must be a valid numeric value")
            );
        }
        Ok(())
    }

    #[test]
    fn test_group_sanitize_name() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "test;group")?; // Invalid characters should be sanitized
        let result = group(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_group_get_group_info_parsing() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testgroup")?;
        let module = group(&lua, params)?;

        // Mock SSH
        let ssh = lua.create_table()?;
        ssh.set(
            "cmdq",
            lua.create_function(|lua, (_self, cmd): (Table, String)| {
                let result = lua.create_table()?;
                result.set("exit_code", 0)?;
                if cmd.contains("getent group") && cmd.contains("testgroup") {
                    // Return group info: name:password:gid:members
                    result.set("stdout", "testgroup:x:1000:user1,user2\n")?;
                } else {
                    result.set("stdout", "")?;
                }
                Ok(result)
            })?,
        )?;
        module.set("ssh", ssh)?;

        let get_group_info: mlua::Function = module.get("get_group_info")?;
        let info: Table = get_group_info.call(module)?;

        assert_eq!(info.get::<String>("name")?, "testgroup");
        assert_eq!(info.get::<String>("gid")?, "1000");
        let members: Vec<String> = info.get("members")?;
        assert_eq!(members, vec!["user1", "user2"]);

        Ok(())
    }

    #[test]
    fn test_group_dry_run_create() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "newgroup")?;
        params.set("state", "present")?;
        let module = group(&lua, params)?;

        // Mock SSH - group doesn't exist
        let ssh = lua.create_table()?;
        ssh.set(
            "cmdq",
            lua.create_function(|lua, (_self, cmd): (Table, String)| {
                let result = lua.create_table()?;
                if cmd.contains("getent group") {
                    result.set("exit_code", 2)?; // Group not found
                } else {
                    result.set("exit_code", 0)?;
                }
                result.set("stdout", "")?;
                Ok(result)
            })?,
        )?;

        let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let changed_clone = changed.clone();
        ssh.set(
            "set_changed",
            lua.create_function(move |_, (_self, val): (Table, bool)| {
                changed_clone.store(val, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })?,
        )?;

        module.set("ssh", ssh)?;

        let dry_run: mlua::Function = module.get("dry_run")?;
        dry_run.call::<()>(module)?;

        // Should be changed because group doesn't exist and we want it present
        assert!(changed.load(std::sync::atomic::Ordering::SeqCst));

        Ok(())
    }

    #[test]
    fn test_group_dry_run_delete() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "existinggroup")?;
        params.set("state", "absent")?;
        let module = group(&lua, params)?;

        // Mock SSH - group exists
        let ssh = lua.create_table()?;
        ssh.set(
            "cmdq",
            lua.create_function(|lua, (_self, cmd): (Table, String)| {
                let result = lua.create_table()?;
                result.set("exit_code", 0)?;
                if cmd.contains("getent group") {
                    result.set("stdout", "existinggroup:x:1000:\n")?; // Group exists
                } else {
                    result.set("stdout", "")?;
                }
                Ok(result)
            })?,
        )?;

        let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let changed_clone = changed.clone();
        ssh.set(
            "set_changed",
            lua.create_function(move |_, (_self, val): (Table, bool)| {
                changed_clone.store(val, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })?,
        )?;

        module.set("ssh", ssh)?;

        let dry_run: mlua::Function = module.get("dry_run")?;
        dry_run.call::<()>(module)?;

        // Should be changed because group exists and we want it absent
        assert!(changed.load(std::sync::atomic::Ordering::SeqCst));

        Ok(())
    }
}
