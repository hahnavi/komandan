use mlua::{chunk, ExternalResult, Lua, Table};

pub fn systemd_service(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.name == nil then
                error("name is required")
            end

            local module = $base_module:new({ name = "systemd_service" })

            module.params = $params

            module.run = function(self)
                local opts = ""
                if self.params.force == true then
                    opts = "--force"
                end

                if self.params.daemon_reload == true then
                    self.ssh:cmd("systemctl daemon-reload")
                end

                if self.params.enabled == true then
                    local enabled = self.ssh:cmd("systemctl is-enabled " .. self.params.name).stdout
                    if enabled == "enabled" then
                        return
                    else
                        self.ssh:cmd("systemctl enable " .. self.params.name .. " " .. opts)
                    end
                elseif self.params.enabled == false then
                    local enabled = self.ssh:cmd("systemctl is-enabled " .. self.params.name).stdout
                    if enabled == "disabled" then
                        return
                    else
                        self.ssh:cmd("systemctl disable " .. self.params.name .. " " .. opts)
                    end
                end

                if self.params.state == "started" then
                    local state = self.ssh:cmd("systemctl is-active " .. self.params.name).stdout
                    if state == "active" then
                        return
                    else
                        self.ssh:cmd("systemctl start " .. self.params.name)
                    end
                elseif self.params.state == "stopped" then
                    local state = self.ssh:cmd("systemctl is-active " .. self.params.name).stdout
                    if state == "inactive" then
                        return
                    else
                        self.ssh:cmd("systemctl stop " .. self.params.name)
                    end
                elseif self.params.state == "reloaded" then
                    self.ssh:cmd("systemctl reload " .. self.params.name)
                elseif self.params.state == "restarted" then
                    self.ssh:cmd("systemctl restart " .. self.params.name)
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
    fn test_systemd_service_name_required() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();

        let result = systemd_service(&lua, params);
        assert!(result.is_err());
    }

    #[test]
    fn test_systemd_service_name_provided() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("name", "test_service").unwrap();

        let result = systemd_service(&lua, params);
        assert!(result.is_ok());
    }
}
