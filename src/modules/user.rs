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

            module.is_exists = function(self)
                local result = self.ssh:cmdq("id -u " .. self.params.name .. " >/dev/null 2>&1")
                return result.exit_code == 0
            end

            module.get_user_info = function(self)
                local result = self.ssh:cmdq("getent passwd " .. self.params.name)
                if result.exit_code ~= 0 then
                    return nil
                end
                local parts = {}
                for s in result.stdout:gmatch("[^:]+") do
                    table.insert(parts, s)
                end
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
                local result = self.ssh:cmdq("groups " .. self.params.name)
                if result.exit_code ~= 0 then
                    return {}
                end
                local groups_str = result.stdout:match(":%s*(.*)")
                if groups_str then
                    local groups = {}
                    for g in groups_str:gmatch("[^%s]+") do
                        table.insert(groups, g)
                    end
                    return groups
                end
                return {}
            end

            module.dry_run = function(self)
                local is_exists = self:is_exists()
                local changed = false

                if self.params.state == "absent" then
                    if is_exists then
                        changed = true
                    end
                elseif self.params.state == "present" then
                    if not is_exists then
                        changed = true
                    else
                        local current_info = self:get_user_info()
                        local current_groups = self:get_user_groups()

                        if self.params.uid ~= nil and current_info.uid ~= tostring(self.params.uid) then changed = true end
                        if self.params.group ~= nil then
                            local current_gid_result = self.ssh:cmdq("id -g -n " .. self.params.name)
                            if current_gid_result.exit_code == 0 and current_gid_result.stdout:gsub("%%s+", "") ~= self.params.group then changed = true end
                        end
                        if self.params.home ~= nil and current_info.home ~= self.params.home then changed = true end
                        if self.params.shell ~= nil and current_info.shell ~= self.params.shell then changed = true end
                        if self.params.password ~= nil and current_info.password ~= self.params.password then changed = true end

                        if self.params.groups ~= nil then
                            local desired_groups = {{}}
                            for _, g in ipairs(self.params.groups) do desired_groups[g] = true end
                            for _, g in ipairs(current_groups) do
                                if not desired_groups[g] then
                                    changed = true
                                    break
                                end
                            end
                            if not changed then
                                for _, g in ipairs(self.params.groups) do
                                    local found = false
                                    for _, cg in ipairs(current_groups) do
                                        if g == cg then
                                            found = true
                                            break
                                        end
                                    end
                                    if not found then
                                        changed = true
                                        break
                                    end
                                end
                            end
                        end
                    end
                end
                self.ssh:set_changed(changed)
            end

            module.run = function(self)
                local is_exists = self:is_exists()
                local changed = false

                if self.params.state == "absent" then
                    if is_exists then
                        local cmd = "userdel"
                        if self.params.remove == true then cmd = cmd .. " -r" end
                        if self.params.force == true then cmd = cmd .. " -f" end
                        cmd = cmd .. " " .. self.params.name
                        self.ssh:cmdq(cmd)
                        changed = true
                    else
                        self.ssh:set_changed(false)
                    end
                elseif self.params.state == "present" then
                    if not is_exists then
                        local cmd = "useradd"
                        if self.params.uid ~= nil then cmd = cmd .. " --uid " .. self.params.uid end
                        if self.params.group ~= nil then cmd = cmd .. " --gid " .. self.params.group end
                        if self.params.groups ~= nil then cmd = cmd .. " --groups " .. table.concat(self.params.groups, ",") end
                        if self.params.home ~= nil then cmd = cmd .. " --home-dir " .. self.params.home end
                        if self.params.shell ~= nil then cmd = cmd .. " --shell " .. self.params.shell end
                        if self.params.password ~= nil then cmd = cmd .. " --password " .. self.params.password end
                        if self.params.system == true then cmd = cmd .. " --system" end
                        if self.params.create_home == true then cmd = cmd .. " --create-home" end
                        cmd = cmd .. " " .. self.params.name
                        self.ssh:cmdq(cmd)
                        changed = true
                    else
                        local current_info = self:get_user_info()
                        local current_groups = self:get_user_groups()
                        local usermod_cmd = "usermod"
                        local usermod_needed = false

                        if self.params.uid ~= nil and current_info.uid ~= tostring(self.params.uid) then
                            usermod_cmd = usermod_cmd .. " --uid " .. self.params.uid
                            usermod_needed = true
                        end
                        if self.params.group ~= nil then
                            local current_gid_result = self.ssh:cmdq("id -g -n " .. self.params.name)
                            if current_gid_result.exit_code == 0 and current_gid_result.stdout:gsub("%%s+", "") ~= self.params.group then
                                usermod_cmd = usermod_cmd .. " --gid " .. self.params.group
                                usermod_needed = true
                            end
                        end
                        if self.params.home ~= nil and current_info.home ~= self.params.home then
                            usermod_cmd = usermod_cmd .. " --home " .. self.params.home
                            usermod_needed = true
                        end
                        if self.params.shell ~= nil and current_info.shell ~= self.params.shell then
                            usermod_cmd = usermod_cmd .. " --shell " .. self.params.shell
                            usermod_needed = true
                        end
                        if self.params.password ~= nil and current_info.password ~= self.params.password then
                            usermod_cmd = usermod_cmd .. " --password " .. self.params.password
                            usermod_needed = true
                        end

                        if self.params.groups ~= nil then
                            local desired_groups_str = table.concat(self.params.groups, ",")
                            local current_groups_str = table.concat(current_groups, ",")
                            if desired_groups_str ~= current_groups_str then
                                usermod_cmd = usermod_cmd .. " --groups " .. desired_groups_str
                                usermod_needed = true
                            end
                        end

                        if usermod_needed then
                            usermod_cmd = usermod_cmd .. " " .. self.params.name
                            self.ssh:cmdq(usermod_cmd)
                            changed = true
                        else
                            self.ssh:set_changed(false)
                        end
                    end
                end
                self.ssh:set_changed(changed)
            end

            return module
            })
        .set_name("user")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
