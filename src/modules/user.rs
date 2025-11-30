use mlua::{ExternalResult, Lua, Table, chunk};

pub fn user(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.name == nil then
                error("'name' parameter is required")
            end

            local valid_states = {
                present = true,
                absent = true,
            }

            if params.state ~= nil and not valid_states[params.state] then
                error("Invalid state: " .. params.state .. ". Valid states are: present, absent.")
            end

            params.state = params.state or "present"

            local module = $base_module:new({ name = "user" })

            module.params = $params

            local function shell_escape(s)
                if s == nil then return "''" end
                return "'" .. string.gsub(s, "'", "'\\''") .. "'"
            end

            local function split(s, delimiter)
                local result = {}
                for match in (s..delimiter):gmatch("(.-)"..delimiter) do
                    table.insert(result, match)
                end
                return result
            end

            module.is_exists = function(self)
                local result = self.ssh:cmdq("id -u " .. shell_escape(self.params.name) .. " >/dev/null 2>&1")
                return result.exit_code == 0
            end

            module.get_user_info = function(self)
                local result = self.ssh:cmdq("getent passwd " .. shell_escape(self.params.name))
                if result.exit_code ~= 0 then
                    return nil
                end
                local stdout = result.stdout:gsub("%s+$", "")
                local parts = split(stdout, ":")
                return {
                    name = parts[1],
                    password = parts[2],
                    uid = parts[3],
                    gid = parts[4],
                    gecos = parts[5],
                    home = parts[6],
                    shell = parts[7],
                }
            end

            module.get_user_groups = function(self)
                local result = self.ssh:cmdq("id -Gn " .. shell_escape(self.params.name))
                if result.exit_code ~= 0 then
                    return {}
                end
                local groups = {}
                for g in result.stdout:gmatch("%S+") do
                    table.insert(groups, g)
                end
                return groups
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
                        local current_info = self:get_user_info()
                        local current_groups = self:get_user_groups()

                        if self.params.uid ~= nil and current_info.uid ~= tostring(self.params.uid) then
                            self.ssh:set_changed(true)
                        end
                        if self.params.group ~= nil then
                            local current_gid_result = self.ssh:cmdq("id -g -n " .. shell_escape(self.params.name))
                            if current_gid_result.exit_code == 0 and current_gid_result.stdout:gsub("%s+", "") ~= self.params.group then
                                self.ssh:set_changed(true)
                            end
                        end
                        if self.params.home ~= nil and current_info.home ~= self.params.home then
                            self.ssh:set_changed(true)
                        end
                        if self.params.shell ~= nil and current_info.shell ~= self.params.shell then
                            self.ssh:set_changed(true)
                        end
                        if self.params.password ~= nil and current_info.password ~= self.params.password then
                            self.ssh:set_changed(true)
                        end

                        if self.params.groups ~= nil then
                            local desired_groups = {}
                            for _, g in ipairs(self.params.groups) do desired_groups[g] = true end
                            
                            local current_groups_set = {}
                            for _, g in ipairs(current_groups) do current_groups_set[g] = true end

                            -- Check if all desired groups are present
                            for g, _ in pairs(desired_groups) do
                                if not current_groups_set[g] then
                                    self.ssh:set_changed(true)
                                    return
                                end
                            end

                            -- Check if there are extra groups (if we want exact match, which usermod -G usually implies replacement)
                            -- usermod -G replaces the list of supplementary groups.
                            for g, _ in pairs(current_groups_set) do
                                if not desired_groups[g] then
                                    self.ssh:set_changed(true)
                                    return
                                end
                            end
                        end
                    end
                end
            end

            module.run = function(self)
                local is_exists = self:is_exists()

                if self.params.state == "absent" then
                    if is_exists then
                        local cmd = "userdel"
                        if self.params.remove == true then cmd = cmd .. " -r" end
                        if self.params.force == true then cmd = cmd .. " -f" end
                        cmd = cmd .. " " .. shell_escape(self.params.name)
                        self.ssh:cmdq(cmd)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.state == "present" then
                    if not is_exists then
                        local cmd = "useradd"
                        if self.params.uid ~= nil then cmd = cmd .. " --uid " .. shell_escape(tostring(self.params.uid)) end
                        if self.params.group ~= nil then cmd = cmd .. " --gid " .. shell_escape(self.params.group) end
                        if self.params.groups ~= nil then 
                            local groups_str = table.concat(self.params.groups, ",")
                            cmd = cmd .. " --groups " .. shell_escape(groups_str) 
                        end
                        if self.params.home ~= nil then cmd = cmd .. " --home-dir " .. shell_escape(self.params.home) end
                        if self.params.shell ~= nil then cmd = cmd .. " --shell " .. shell_escape(self.params.shell) end
                        if self.params.password ~= nil then cmd = cmd .. " --password " .. shell_escape(self.params.password) end
                        if self.params.system == true then cmd = cmd .. " --system" end
                        if self.params.create_home == true then cmd = cmd .. " --create-home" end
                        cmd = cmd .. " " .. shell_escape(self.params.name)
                        self.ssh:cmdq(cmd)
                        self.ssh:set_changed(true)
                    else
                        local current_info = self:get_user_info()
                        local current_groups = self:get_user_groups()
                        local usermod_cmd = "usermod"
                        local usermod_needed = false

                        if self.params.uid ~= nil and current_info.uid ~= tostring(self.params.uid) then
                            usermod_cmd = usermod_cmd .. " --uid " .. shell_escape(tostring(self.params.uid))
                            usermod_needed = true
                        end
                        if self.params.group ~= nil then
                            local current_gid_result = self.ssh:cmdq("id -g -n " .. shell_escape(self.params.name))
                            if current_gid_result.exit_code == 0 and current_gid_result.stdout:gsub("%s+", "") ~= self.params.group then
                                usermod_cmd = usermod_cmd .. " --gid " .. shell_escape(self.params.group)
                                usermod_needed = true
                            end
                        end
                        if self.params.home ~= nil and current_info.home ~= self.params.home then
                            usermod_cmd = usermod_cmd .. " --home " .. shell_escape(self.params.home)
                            usermod_needed = true
                        end
                        if self.params.shell ~= nil and current_info.shell ~= self.params.shell then
                            usermod_cmd = usermod_cmd .. " --shell " .. shell_escape(self.params.shell)
                            usermod_needed = true
                        end
                        if self.params.password ~= nil and current_info.password ~= self.params.password then
                            usermod_cmd = usermod_cmd .. " --password " .. shell_escape(self.params.password)
                            usermod_needed = true
                        end

                        if self.params.groups ~= nil then
                            local desired_groups = {}
                            for _, g in ipairs(self.params.groups) do desired_groups[g] = true end
                            
                            local current_groups_set = {}
                            for _, g in ipairs(current_groups) do current_groups_set[g] = true end

                            local groups_changed = false
                            for g, _ in pairs(desired_groups) do
                                if not current_groups_set[g] then
                                    groups_changed = true
                                    break
                                end
                            end
                            if not groups_changed then
                                for g, _ in pairs(current_groups_set) do
                                    if not desired_groups[g] then
                                        groups_changed = true
                                        break
                                    end
                                end
                            end

                            if groups_changed then
                                local desired_groups_str = table.concat(self.params.groups, ",")
                                usermod_cmd = usermod_cmd .. " --groups " .. shell_escape(desired_groups_str)
                                usermod_needed = true
                            end
                        end

                        if usermod_needed then
                            usermod_cmd = usermod_cmd .. " " .. shell_escape(self.params.name)
                            self.ssh:cmdq(usermod_cmd)
                            self.ssh:set_changed(true)
                        end
                    end
                end
            end

            return module
            })
        .set_name("user")
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
    fn test_user_name_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        let result = user(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("'name' parameter is required"));
        }
        Ok(())
    }

    #[test]
    fn test_user_valid_name() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testuser")?;
        let result = user(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_user_get_user_info_parsing() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testuser")?;
        let module = user(&lua, params)?;

        // Mock SSH
        let ssh = lua.create_table()?;
        ssh.set("cmdq", lua.create_function(|lua, (_self, cmd): (Table, String)| {
            let result = lua.create_table()?;
            result.set("exit_code", 0)?;
            if cmd.contains("getent passwd") {
                // Return output with empty fields (e.g. empty GECOS) and trailing newline
                // testuser:x:1000:1000::/home/testuser:/bin/bash\n
                result.set("stdout", "testuser:x:1000:1000::/home/testuser:/bin/bash\n")?;
            } else {
                result.set("stdout", "")?;
            }
            Ok(result)
        })?)?;
        module.set("ssh", ssh)?;

        let get_user_info: mlua::Function = module.get("get_user_info")?;
        let info: Table = get_user_info.call(module)?;

        assert_eq!(info.get::<String>("name")?, "testuser");
        assert_eq!(info.get::<String>("uid")?, "1000");
        assert_eq!(info.get::<String>("gecos")?, "");
        assert_eq!(info.get::<String>("home")?, "/home/testuser");
        assert_eq!(info.get::<String>("shell")?, "/bin/bash");

        Ok(())
    }

    #[test]
    fn test_user_group_comparison() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "testuser")?;
        params.set("groups", vec!["group1", "group2"])?;
        let module = user(&lua, params)?;

        // Mock SSH
        let ssh = lua.create_table()?;
        ssh.set("cmdq", lua.create_function(|lua, (_self, cmd): (Table, String)| {
            let result = lua.create_table()?;
            result.set("exit_code", 0)?;
            if cmd.contains("id -Gn") {
                // Return groups in different order
                result.set("stdout", "group2 group1")?;
            } else if cmd.contains("id -u") {
                result.set("exit_code", 0)?;
            } else if cmd.contains("getent passwd") {
                result.set("stdout", "testuser:x:1000:1000::/home/testuser:/bin/bash")?;
            } else {
                result.set("stdout", "")?;
            }
            Ok(result)
        })?)?;
        
        let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let changed_clone = changed.clone();
        ssh.set("set_changed", lua.create_function(move |_, (_self, val): (Table, bool)| {
            changed_clone.store(val, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?)?;
        
        ssh.set("get_changed", lua.create_function(|_, _self: Table| {
            Ok(false)
        })?)?;

        module.set("ssh", ssh)?;

        let dry_run: mlua::Function = module.get("dry_run")?;
        dry_run.call::<()>(module)?;

        // Should not be changed because groups are same set
        assert!(!changed.load(std::sync::atomic::Ordering::SeqCst));

        Ok(())
    }
}
