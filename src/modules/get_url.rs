use mlua::{ExternalResult, Lua, Table, chunk};

pub fn get_url(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.url == nil then
                error("'url' parameter is required")
            end

            if params.dest == nil then
                error("'dest' parameter is required")
            end

            params.force = params.force or false

            local module = $base_module:new({ name = "get_url" })

            module.params = $params

            module.is_exists = function(self)
                local result = self.ssh:cmdq("test -f " .. self.params.dest)
                return result.exit_code == 0
            end

            module.dry_run = function(self)
                local is_exists = self:is_exists()
                if is_exists and not self.params.force then
                    self.ssh:set_changed(false)
                end
            end

            module.run = function(self)
                local is_exists = self:is_exists()
                if is_exists and not self.params.force then
                    self.ssh:set_changed(false)
                else
                    self.ssh:cmdq("wget -O " .. self.params.dest .. " " .. self.params.url)
                end
            end

            return module
        })
        .set_name("get_url")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
