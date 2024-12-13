use mlua::{chunk, Error::RuntimeError, ExternalResult, Lua, Table};

pub fn apt(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let package = match params.get::<String>("package") {
        Ok(p) => p,
        Err(_) => return Err(RuntimeError("package is required".into())),
    };
    let action = params
        .get::<String>("action")
        .unwrap_or(String::from("install"));
    let update_cache = params.get::<bool>("update_cache").unwrap_or(false);
    let install_recommends = params.get::<bool>("install_recommends").unwrap_or(true);

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "apt" })

            function module:run()
                if $update_cache then
                    module.ssh:cmd("apt update")
                end

                local install_opts = ""
                if not $install_recommends then
                    install_opts = install_opts .. " --no-install-recommends"
                end

                if $action == "install" then
                    module.ssh:cmd("apt install -y " .. $package .. install_opts)
                elseif $action == "remove" then
                    module.ssh:cmd("apt remove -y " .. $package)
                elseif $action == "purge" then
                    module.ssh:cmd("apt purge -y " .. $package)
                elseif $action == "upgrade" then
                    module.ssh:cmd("apt upgrade -y")
                elseif $action == "autoremove" then
                    module.ssh:cmd("apt autoremove -y")
                end
            end

            return module
        })
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
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: package is required"
        );
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
