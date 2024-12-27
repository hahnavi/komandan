use mlua::{chunk, ExternalResult, Lua, Table};

pub fn apt(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            if params.update_cache == nil then
                params.update_cache = false
            end

            if params.package == nil and params.update_cache == false then
                error("package is required")
            end

            if params.install_recommends == nil then
                params.install_recommends = true
            end

            if params.action == nil then
                params.action = "install"
            end

            local module = $base_module:new({ name = "apt" })

            module.params = $params

            module.run = function(self)
                if self.params.update_cache then
                    self.ssh:cmd("apt update")
                end

                if self.params.package == nill then
                    return
                end

                local install_opts = ""
                if not self.params.install_recommends then
                    install_opts = install_opts .. " --no-install-recommends"
                end

                if self.params.action == "install" then
                    self.ssh:cmd("apt install -y " .. self.params.package .. install_opts)
                elseif self.params.action == "remove" then
                    self.ssh:cmd("apt remove -y " .. self.params.package)
                elseif self.params.action == "purge" then
                    self.ssh:cmd("apt purge -y " .. self.params.package)
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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_package_required() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        let result = apt(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("package is required"));
    }

    #[test]
    fn test_valid_package() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("package", "vim").unwrap();
        let result = apt(&lua, params);
        assert!(result.is_ok());
    }
}
