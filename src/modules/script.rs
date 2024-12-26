use mlua::{chunk, Lua, Table};
use rand::{distributions::Alphanumeric, Rng};

pub fn script(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            if params.script == nil and params.from_file == nil then
                error("script or from_file parameter is required")
            end

            if params.script ~= nil and params.from_file ~= nil then
                error("script and from_file parameters cannot be used together")
            end

            local module = $base_module:new({ name = "script" })

            module.params = $params
            module.random_file_name = $random_file_name

            module.run = function(self)
                local tmpdir = self.ssh:get_tmpdir()
                self.remote_path = tmpdir .. "/." .. self.random_file_name

                if self.params.script ~= nil then
                    self.ssh:write_remote_file(self.remote_path, self.params.script)
                elseif self.params.from_file ~= nil then
                    self.ssh:upload(self.params.from_file, self.remote_path)
                end

                if self.params.interpreter ~= nil then
                    self.ssh:cmd(self.params.interpreter .. " " .. self.remote_path)
                else
                    self.ssh:chmod(self.remote_path, "+x")
                    self.ssh:cmd(self.remote_path)
                end
            end

            module.cleanup = function(self)
                self.ssh:cmd("rm " .. self.remote_path)
            end

            return module
        })
        .eval::<Table>()?;

    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_script_or_from_file_required() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        let result = script(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("script or from_file parameter is required"));
    }

    #[test]
    fn test_script_and_from_file_exclusive() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("script", "echo hello").unwrap();
        params.set("from_file", "examples/run_script.lua").unwrap();
        let result = script(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("script and from_file parameters cannot be used together"));
    }

    #[test]
    fn test_script() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("script", "echo hello").unwrap();
        params.set("interpreter", "bash").unwrap();
        let result = script(&lua, params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_from_file() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("from_file", "examples/run_script.lua").unwrap();
        let result = script(&lua, params);
        assert!(result.is_ok());
    }
}
