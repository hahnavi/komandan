use mlua::{chunk, ExternalResult, Lua, Table};

pub fn upload(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let src = params.get::<String>("src")?;
    let dst = params.get::<String>("dst")?;

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "upload" })

            function module:run()
                module.ssh:upload($src, $dst)
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
    fn test_upload_success() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("src", "examples/run_script.lua").unwrap();
        params.set("dst", "/tmp/test_upload.lua").unwrap();
        let result = upload(&lua, params);
        assert!(result.is_ok());
    }
}
