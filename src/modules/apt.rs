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

            if params.install_opts == nil then
                params.install_opts = ""
            end
            if not params.install_recommends then
                params.install_opts = params.install_opts .. " --no-install-recommends"
            end

            local function sanitize_string(input)
                if type(input) ~= "string" then
                    return nil -- Ensure input is a string
                end
                -- Allow alphanumeric, -, _, =, ., +, and space (for opts)
                return input:gsub("[^%w%-_=%.%+ ]", "")
            end

            local function sanitize_package_param(param)
                if type(param) == "string" then
                    return sanitize_string(param)
                elseif type(param) == "table" then
                    local sanitized_packages = {}
                    for _, pkg in ipairs(param) do
                        if type(pkg) == "string" then
                            local sanitized_pkg = sanitize_string(pkg)
                            table.insert(sanitized_packages, sanitized_pkg)
                        end
                    end
                    return sanitized_packages
                else
                    return nil
                end
            end

            params.package = sanitize_package_param(params.package)
            params.install_opts = sanitize_string(params.install_opts)

            local module = $base_module:new({ name = "apt" })

            module.params = $params

            module.update_cache = function(self)
                local update_result = self.ssh:cmd("apt update")
                if update_result.exit_code == 0 and update_result.stdout:match("Get:") then
                    self.ssh:set_changed(true)
                end
            end

            module.is_installed = function(self)
                if self.params.package == nil then
                    return false
                end

                if type(self.params.package) == "string" then
                    local pkg_check = self.ssh:cmdq("dpkg-query -W -f='${Status}' " .. self.params.package .. " 2>/dev/null | grep -q 'ok installed'")
                    return pkg_check.exit_code == 0
                elseif type(self.params.package) == "table" then
                    -- For install: return true only if ALL are installed
                    -- For remove: return true if ANY is installed (so we know to run remove)

                    local all_installed = true
                    local any_installed = false

                    for _, pkg in ipairs(self.params.package) do
                        local pkg_check = self.ssh:cmdq("dpkg-query -W -f='${Status}' " .. pkg .. " 2>/dev/null | grep -q 'ok installed'")
                        if pkg_check.exit_code == 0 then
                            any_installed = true
                        else
                            all_installed = false
                        end
                    end

                    if self.params.action == "remove" or self.params.action == "purge" then
                        return any_installed
                    else
                        return all_installed
                    end
                else
                    return false
                end
            end

            module.package_list_to_string = function(package_list)
                if type(package_list) == "string" then
                    return package_list
                elseif type(package_list) == "table" then
                    return table.concat(package_list, " ")
                else
                    error("Invalid package.")
                end
            end

            module.dry_run = function(self)
                if self.params.update_cache then
                    self:update_cache()
                end

                local installed = self:is_installed()
                local packages_str = self.package_list_to_string(self.params.package)

                if self.params.action == "install" then
                    if not installed then
                        self.ssh:cmd("apt -s install " .. packages_str .. " " .. self.params.install_opts)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "remove" then
                    if installed then
                        self.ssh:cmd("apt -s remove " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "purge" then
                    if installed then
                        self.ssh:cmd("apt -s purge " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "upgrade" then
                    local check = self.ssh:cmd("apt -s upgrade | grep -q '0 upgraded, 0 newly installed, 0 to remove'")
                    if check.exit_code ~= 0 then
                         self.ssh:cmd("apt -s upgrade")
                         self.ssh:set_changed(true)
                    end
                elseif self.params.action == "autoremove" then
                    local check = self.ssh:cmd("apt -s autoremove | grep -q '0 upgraded, 0 newly installed, 0 to remove'")
                    if check.exit_code ~= 0 then
                        self.ssh:cmd("apt -s autoremove")
                        self.ssh:set_changed(true)
                    end
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
                local packages_str = self.package_list_to_string(self.params.package)

                if self.params.action == "install" then
                    if not installed then
                        self.ssh:cmd("apt install -y " .. packages_str .. " " .. self.params.install_opts)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "remove" then
                    if installed then
                        self.ssh:cmd("apt remove -y " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "purge" then
                    if installed then
                        self.ssh:cmd("apt purge -y " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "upgrade" then
                    local check = self.ssh:cmd("apt -s upgrade | grep -q '0 upgraded, 0 newly installed, 0 to remove'")
                    if check.exit_code ~= 0 then
                        self.ssh:cmd("apt upgrade -y")
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "autoremove" then
                    local check = self.ssh:cmd("apt -s autoremove | grep -q '0 upgraded, 0 newly installed, 0 to remove'")
                    if check.exit_code ~= 0 then
                        self.ssh:cmd("apt autoremove -y")
                        self.ssh:set_changed(true)
                    end
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
    fn test_apt_package_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("action", "install")?;
        let result = apt(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("package is required"));
        }
        Ok(())
    }

    #[test]
    fn test_apt_valid_package() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("package", "vim")?;
        let result = apt(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_apt_sanitization() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("package", "g++")?;
        let module = apt(&lua, params)?;
        let params: Table = module.get("params")?;
        let package: String = params.get("package")?;
        assert_eq!(package, "g++");

        let params = lua.create_table()?;
        params.set("package", "python3.8")?;
        let module = apt(&lua, params)?;
        let params: Table = module.get("params")?;
        let package: String = params.get("package")?;
        assert_eq!(package, "python3.8");

        Ok(())
    }
}
