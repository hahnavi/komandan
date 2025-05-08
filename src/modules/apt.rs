use mlua::{ExternalResult, Lua, Table, chunk};

pub fn apt(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.update_cache == nil then
                params.update_cache = false
            end

            local valid_actions = {
                install = true,
                remove = true,
                purge = true,
                upgrade = true,
                autoremove = true
            }

            if params.action ~= nil and not valid_actions[params.action] then
                error("Invalid action: " .. params.action .. ". Valid actions are: install, remove, purge, upgrade, autoremove.")
            end

            if (params.action == "install" or params.action == "remove" or params.action == "purge") and params.package == nil then
                error("package is required")
            end

            if params.package ~= nil and params.action == nil then
                params.action = "install"
            end

            if params.install_recommends == nil then
                params.install_recommends = true
            end

            params.install_opts = ""
            if not params.install_recommends then
                params.install_opts = params.install_opts .. " --no-install-recommends"
            end

            local function sanitize(input)
                if type(input) ~= "string" then
                    return nil -- Ensure input is a string
                end
                return input:gsub("[^%w%-_]", "")
            end

            params.package = sanitize(params.package)
            params.install_opts = sanitize(params.install_opts)

            local module = $base_module:new({ name = "apt" })

            module.params = $params

            module.update_cache = function(self)
                local update_result = self.ssh:cmd("apt update")
                if update_result.exit_code == 0 and not update_result.stdout:match("Get:") then
                    self.ssh:set_changed(false)
                end
            end

            module.is_installed = function(self)
                if self.params.package == nil then
                    return false
                end

                local pkg_check = self.ssh:cmdq("dpkg-query -W -f='${Status}' " .. self.params.package .. " 2>/dev/null | grep -q 'ok installed'")
                if pkg_check.exit_code == 0 then
                    return true
                else
                    return false
                end
            end

            module.dry_run = function(self)
                if self.params.update_cache then
                    self:update_cache()
                end

                local installed = self:is_installed()

                if self.params.action == "install" then
                    self.ssh:cmd("apt -s install " .. self.params.package .. self.params.install_opts)
                    if installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "remove" then
                    self.ssh:cmd("apt -s remove " .. self.params.package)
                    if not installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "purge" then
                    self.ssh:cmd("apt -s purge " .. self.params.package)
                    if not installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "upgrade" then
                    self.ssh:cmd("apt -s upgrade")
                elseif self.params.action == "autoremove" then
                    self.ssh:cmd("apt -s autoremove")
                end
            end

            module.run = function(self)
                if self.params.update_cache then
                    self:update_cache()
                end

                if self.params.package == nil then
                    return
                end

                local installed = self:is_installed()

                if self.params.action == "install" then
                    self.ssh:cmd("apt install -y " .. self.params.package .. self.params.install_opts)
                    if installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "remove" then
                    self.ssh:cmd("apt remove -y " .. self.params.package)
                    if not installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "purge" then
                    self.ssh:cmd("apt purge -y " .. self.params.package)
                    if not installed then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "upgrade" then
                    self.ssh:cmd("apt upgrade -y")
                elseif self.params.action == "autoremove" then
                    self.ssh:cmd("apt autoremove -y")
                end
            end

            return module
        })
        .set_name("apt")
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
    fn test_apt_package_required() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("action", "install").unwrap();
        let result = apt(&lua, params);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("package is required")
        );
    }

    #[test]
    fn test_apt_valid_package() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("package", "vim").unwrap();
        let result = apt(&lua, params);
        assert!(result.is_ok());
    }
}
