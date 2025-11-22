use mlua::{ExternalResult, Lua, Table, chunk};

pub fn systemd_service(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.name == nil then
                error("name is required")
            end

            local valid_actions = {
                start = true,
                stop = true,
                restart = true,
                reload = true,
                enable = true,
                disable = true,
            }

            if params.action ~= nil and not valid_actions[params.action] then
                error("Invalid action: " .. params.action .. ". Valid actions are: start, stop, restart, reload, enable, and disable.")
            end

            params.action = params.action or "start"

            local module = $base_module:new({ name = "systemd_service" })

            module.params = $params

            module.dry_run = function(self)
                if self.params.action == "start" then
                    local state = self.ssh:cmdq("systemctl is-active " .. self.params.name).stdout
                    if state ~= "active" then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "stop" then
                    local state = self.ssh:cmdq("systemctl is-active " .. self.params.name).stdout
                    if state == "active" then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "enable" then
                    local enabled = self.ssh:cmdq("systemctl is-enabled " .. self.params.name).stdout
                    if enabled ~= "enabled" then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "disable" then
                    local enabled = self.ssh:cmdq("systemctl is-enabled " .. self.params.name).stdout
                    if enabled == "enabled" then
                        self.ssh:set_changed(true)
                    end
                end
            end

            module.run = function(self)
                local opts = ""
                if self.params.force == true then
                    opts = "--force"
                end

                if self.params.daemon_reload == true then
                    self.ssh:cmd("systemctl daemon-reload")
                end

                if self.params.action == "start" then
                    local state = self.ssh:cmdq("systemctl is-active " .. self.params.name).stdout
                    if state ~= "active" then
                        self.ssh:cmd("systemctl start " .. self.params.name)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "stop" then
                    local state = self.ssh:cmdq("systemctl is-active " .. self.params.name).stdout
                    if state == "active" then
                        self.ssh:cmd("systemctl stop " .. self.params.name)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "reload" then
                    self.ssh:cmd("systemctl reload " .. self.params.name)
                elseif self.params.action == "restart" then
                    self.ssh:cmd("systemctl restart " .. self.params.name)
                elseif self.params.action == "enable" then
                    local enabled = self.ssh:cmdq("systemctl is-enabled " .. self.params.name).stdout
                    if enabled ~= "enabled" then
                        self.ssh:cmd("systemctl enable " .. self.params.name .. " " .. opts)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "disable" then
                    local enabled = self.ssh:cmdq("systemctl is-enabled " .. self.params.name).stdout
                    if enabled == "enabled" then
                        self.ssh:cmd("systemctl disable " .. self.params.name .. " " .. opts)
                        self.ssh:set_changed(true)
                    end
                end
            end

            return module
        })
        .set_name("systemd_service")
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
    fn test_systemd_service_name_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;

        let result = systemd_service(&lua, params);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_systemd_service_name_provided() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("name", "test_service")?;

        let result = systemd_service(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }
}
