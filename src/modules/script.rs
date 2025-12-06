use mlua::{Lua, Table, chunk};
use rand::{Rng, distr::Alphanumeric};

pub fn script(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let random_file_name: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(lua)?;
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
                local script_content = self.params.script
                local use_inline = false
                
                -- Determine if we can execute inline (script < 100KB and not from_file)
                if script_content ~= nil then
                    local script_size = #script_content
                    if script_size < 102400 then -- 100KB = 102400 bytes
                        use_inline = true
                    end
                end

                if use_inline then
                    -- Execute inline using heredoc
                    local interpreter = self.params.interpreter or "sh"
                    local cmd = interpreter .. " <<'SCRIPT_EOF'\n" .. script_content .. "\nSCRIPT_EOF"
                    self.ssh:cmd(cmd)
                else
                    -- Transfer file and execute (for large scripts or from_file)
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

                self.ssh:set_changed(true)
            end

            module.cleanup = function(self)
                -- Only cleanup if created a remote file
                if self.remote_path ~= nil then
                    self.ssh:cmd("rm " .. self.remote_path)
                end
            end

            return module
        })
        .set_name("script")
        .eval::<Table>()?;

    Ok(module)
}

// Tests
#[cfg(test)]
mod tests {
    use crate::create_lua;

    use super::*;

    #[test]
    fn test_script_or_from_file_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        let result = script(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("script or from_file parameter is required")
            );
        }
        Ok(())
    }

    #[test]
    fn test_script_and_from_file_exclusive() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("script", "echo hello")?;
        params.set("from_file", "examples/run_script.lua")?;
        let result = script(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("script and from_file parameters cannot be used together")
            );
        }
        Ok(())
    }

    #[test]
    fn test_script() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("script", "echo hello")?;
        params.set("interpreter", "bash")?;
        let result = script(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_from_file() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("from_file", "examples/run_script.lua")?;
        let result = script(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }
}
