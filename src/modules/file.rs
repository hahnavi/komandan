use mlua::{ExternalResult, Lua, Table, chunk};

pub fn file(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.path == nil then
                error("'path' parameter is required")
            end

            local valid_states = {
                absent = true,
                directory = true,
                file = true,
                link = true,
            }

            if params.state ~= nil and not valid_states[params.state] then
                error("Invalid state: " .. params.state .. ". Valid states are: absent, directory, file, and link.")
            end

            if params.state == "link" and params.src == nil then
                error("'src' parameter is required when state is 'link'")
            end

            params.state = params.state or "file"

            local module = $base_module:new({ name = "file" })

            module.params = $params

            module.is_exists = function(self)
                local result = self.ssh:cmdq("[ -e " .. self.params.path .. " ]")
                if result.exit_code ~= 0 then
                    return false
                end
                return true
            end

            module.get_mode = function(self)
                local result = self.ssh:cmdq("stat -c %a " .. self.params.path)
                if result.exit_code ~= 0 then
                    error(result.stderr)
                end
                return result.stdout
            end

            module.get_owner = function(self)
                local result = self.ssh:cmdq("stat -c %U " .. self.params.path)
                if result.exit_code ~= 0 then
                    error(result.stderr)
                end
                return result.stdout
            end

            module.get_group = function(self)
                local result = self.ssh:cmdq("stat -c %G " .. self.params.path)
                if result.exit_code ~= 0 then
                    error(result.stderr)
                end
                return result.stdout
            end

            module.dry_run = function(self)
                local is_exists = self:is_exists()

                if self.params.state == "absent" then
                    if not is_exists then
                        self.ssh:set_changed(false)
                    end
                    return
                elseif self.params.state == "directory" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.state == "file" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.state == "link" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    end
                end
            end

            module.run = function(self)
                local is_exists = self:is_exists()

                if self.params.state == "absent" then
                    if is_exists then
                        self.ssh:cmdq("rm -rf " .. self.params.path)
                    else
                        self.ssh:set_changed(false)
                    end
                    return
                elseif self.params.state == "directory" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    else
                        self.ssh:cmdq("mkdir -p " .. self.params.path)
                    end
                elseif self.params.state == "file" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    else
                        self.ssh:cmdq("touch " .. self.params.path)
                    end
                elseif self.params.state == "link" then
                    if is_exists then
                        self.ssh:set_changed(false)
                    else
                        self.ssh:cmdq("ln -s " .. self.params.src .. " " .. self.params.path)
                    end
                end

                if self.params.mode ~= nil then
                    self.ssh:cmdq("chmod " .. self.params.mode .. " " .. self.params.path)
                end

                if self.params.owner ~= nil then
                    self.ssh:cmdq("chown " .. self.params.owner .. " " .. self.params.path)
                end

                if self.params.group ~= nil then
                    self.ssh:cmdq("chgrp " .. self.params.group .. " " .. self.params.path)
                end
            end

            return module
        })
        .set_name("file")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
