use mlua::{ExternalResult, Lua, Table, chunk};

pub fn dnf(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.update_cache == nil then
                params.update_cache = false
            end

            local valid_actions = {
                install = true,
                remove = true,
                update = true,
                upgrade = true,
                autoremove = true,
            }

            if params.action ~= nil and not valid_actions[params.action] then
                error("Invalid action: " .. params.action .. ". Valid actions are: install, remove, update, upgrade, autoremove.")
            end

            if (params.action == "install" or params.action == "remove") and params.package == nil then
                error("package is required")
            end

            if params.package ~= nil and params.action == nil then
                params.action = "install"
            end

            if params.install_weak_deps == nil then
                params.install_weak_deps = true
            end

            params.install_opts = ""
            if not params.install_weak_deps then
                params.install_opts = params.install_opts .. "--setopt=install_weak_deps=False"
            end

            local function sanitize_string(input)
                if type(input) ~= "string" then
                    return nil -- Ensure input is a string
                end
                return input:gsub("[^%w%-_=%.%+]", "")
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

            local module = $base_module:new({ name = "dnf" })

            module.params = $params

            module.update_cache = function(self)
                local update_result = self.ssh:cmd("dnf makecache")
                if update_result.exit_code == 0 and update_result.stdout:match("Metadata cache created") then
                    self.ssh:set_changed(true)
                end
            end

            module.is_installed = function(self)
                if self.params.package == nil then
                    return false
                end

                if type(self.params.package) == "string" then
                    local pkg_check = self.ssh:cmdq("dnf repoquery --installed --whatprovides " .. self.params.package .. " 2>/dev/null")
                    return pkg_check.stdout ~= ""
                elseif type(self.params.package) == "table" then
                    local all_installed = true
                    local any_installed = false

                    for _, pkg in ipairs(self.params.package) do
                        local pkg_check = self.ssh:cmdq("dnf repoquery --installed --whatprovides " .. pkg .. " 2>/dev/null")
                        if pkg_check.stdout ~= "" then
                            any_installed = true
                        else
                            all_installed = false
                        end
                    end

                    if self.params.action == "remove" then
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

                if self.params.action == "install" then
                    if not installed then
                        local packages_str = self.package_list_to_string(self.params.package)
                        self.ssh:cmd("dnf --assumeno install " .. packages_str .. " " .. self.params.install_opts)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "remove" then
                    if installed then
                        local packages_str = self.package_list_to_string(self.params.package)
                        self.ssh:cmd("dnf --assumeno remove " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "update" or self.params.action == "upgrade" then
                    local result = self.ssh:cmd("dnf --assumeno upgrade")
                    if result.exit_code == 1 and not result.stdout:match("Nothing to do") then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "autoremove" then
                    local result = self.ssh:cmd("dnf --assumeno autoremove")
                    if result.exit_code == 1 and not result.stdout:match("Nothing to do") then
                        self.ssh:set_changed(true)
                    end
                end
            end

            module.run = function(self)
                if self.params.update_cache then
                    self:update_cache()
                end

                local installed = self:is_installed()

                if self.params.action == "install" then
                    if not installed then
                        local packages_str = self.package_list_to_string(self.params.package)
                        self.ssh:cmd("dnf install -y " .. packages_str .. " " .. self.params.install_opts)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "remove" then
                    if installed then
                        local packages_str = self.package_list_to_string(self.params.package)
                        self.ssh:cmd("dnf remove -y " .. packages_str)
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "update" or self.params.action == "upgrade" then
                    local result = self.ssh:cmd("dnf upgrade -y")
                    if result.exit_code == 0 and not result.stdout:match("Nothing to do") then
                        self.ssh:set_changed(true)
                    end
                elseif self.params.action == "autoremove" then
                    local result = self.ssh:cmd("dnf autoremove -y")
                    if result.exit_code == 0 and not result.stdout:match("Nothing to do") then
                        self.ssh:set_changed(true)
                    end
                end
            end

            return module
        })
        .set_name("dnf")
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
    fn test_dnf_package_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("action", "install")?;
        let result = dnf(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("package is required"));
        }
        Ok(())
    }

    #[test]
    fn test_dnf_valid_package() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("package", "vim")?;
        let result = dnf(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }
    #[test]
    fn test_dnf_sanitization() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("package", "g++")?;
        let module = dnf(&lua, params)?;
        let params: Table = module.get("params")?;
        let package: String = params.get("package")?;
        assert_eq!(package, "g++");

        let params = lua.create_table()?;
        params.set("package", "python3.8")?;
        let module = dnf(&lua, params)?;
        let params: Table = module.get("params")?;
        let package: String = params.get("package")?;
        assert_eq!(package, "python3.8");

        Ok(())
    }
}
