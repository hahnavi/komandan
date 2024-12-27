use mlua::{chunk, ExternalResult, Lua, Table};

pub fn download(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "download" })

            module.params = $params

            module.run = function(self)
                self.ssh:download(self.params.src, self.params.dst)
            end

            return module
        })
        .set_name("download")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_download_success() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("src", "/tmp/test_download.lua").unwrap();
        params.set("dst", "examples/downloaded.lua").unwrap();
        let result = download(&lua, params);
        assert!(result.is_ok());
    }
}
