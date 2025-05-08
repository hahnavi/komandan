use mlua::{ExternalResult, Lua, Table, chunk};

pub fn upload(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "upload" })

            module.params = $params

            module.run = function(self)
                self.ssh:upload(self.params.src, self.params.dst)
            end

            return module
        })
        .set_name("upload")
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
    fn test_upload_success() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("src", "examples/run_script.lua").unwrap();
        params.set("dst", "/tmp/test_upload.lua").unwrap();
        let result = upload(&lua, params);
        assert!(result.is_ok());
    }
}
