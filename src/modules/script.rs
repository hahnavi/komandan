use mlua::{chunk, Error::RuntimeError, Lua, Table, Value};
use rand::{distributions::Alphanumeric, Rng};

pub fn script(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let script = params.get::<Value>("script")?;
    let from_file = params.get::<Value>("from_file")?;

    if script.is_nil() && from_file.is_nil() {
        return Err(RuntimeError(String::from(
            "script or from_file parameter is required",
        )));
    }

    if !script.is_nil() && !from_file.is_nil() {
        return Err(RuntimeError(String::from(
            "script and from_file parameters cannot be used together",
        )));
    }

    let interpreter = params.get::<Value>("interpreter").unwrap();

    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "script" })

            function module:run()
                local tmpdir = module.ssh:get_tmpdir()
                module.remote_path = tmpdir .. "/." .. $random_file_name

                if $script ~= nil then
                    module.ssh:write_remote_file(module.remote_path, $script)
                elseif $from_file ~= nil then
                    module.ssh:upload($from_file, module.remote_path)
                end

                if $interpreter ~= nil then
                    module.ssh:cmd($interpreter .. " " .. module.remote_path)
                else
                    module.ssh:chmod(module.remote_path, "0755")
                    module.ssh:cmd(module.remote_path)
                end
            end

            function module:cleanup()
                local tmpdir = module.ssh:get_tmpdir()
                module.ssh:cmd("rm " .. module.remote_path)
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
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: script or from_file parameter is required"
        );
    }

    #[test]
    fn test_script_and_from_file_exclusive() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("script", "echo hello").unwrap();
        params.set("from_file", "examples/run_script.lua").unwrap();
        let result = script(&lua, params);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: script and from_file parameters cannot be used together"
        );
    }
}
