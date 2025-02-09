use mlua::{chunk, ExternalResult, Lua, Table};

pub fn postgresql_user(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.name == nil then
                error("'name' parameter is required")
            end

            local valid_actions = {
                create = true,
                drop = true,
            }

            if params.action ~= nil and not valid_actions[params.action] then
                error("Invalid action: " .. params.action .. ". Valid actions are: create and drop.")
            end

            params.action = params.action or "create"

            local module = $base_module:new({ name = "postgresql_user" })

            module.params = $params

            module.is_exists = function(self)
                self.ssh:requires("psql")
                local result = self.ssh:cmdq("psql -tAc \"SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = '" .. self.params.name .. "')::int;\"")
                if result.exit_code ~= 0 then
                    error(result.stderr)
                end
                if result.stdout == "1" then
                    return true
                end
                return false
            end

            module.dry_run = function(self)
                if self.params.action == "create" then
                    if self:is_exists() then
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "drop" then
                    if not self:is_exists() then
                        self.ssh:set_changed(false)
                    end
                end
            end

            module.run = function(self)
                local query = ""
                if self.params.action == "create" then
                    query = "CREATE USER " .. self.params.name
                    if self.params.role_attr_flags ~= nil or self.params.password ~= nil then
                        query = query .. " WITH "
                        if self.params.role_attr_flags ~= nil then
                            query = query .. " " .. self.params.role_attr_flags
                        end
                        if self.params.password ~= nil then
                            query = query .. " PASSWORD '" .. self.params.password .. "'"
                        end
                    end
                elseif self.params.action == "drop" then
                    query = "DROP ROLE " .. self.params.name
                end
                query = query .. ";"

                if self.params.action == "create" then
                    if not self:is_exists() then
                        self.ssh:cmdq("psql -c \"" .. query .. "\"")
                    else
                        self.ssh:set_changed(false)
                    end
                elseif self.params.action == "drop" then
                    if self:is_exists() then
                        self.ssh:cmdq("psql -c \"" .. query .. "\"")
                    else
                        self.ssh:set_changed(false)
                    end
                end
            end

            return module
        })
        .set_name("postgresql_user")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn setup_lua() -> Lua {
        Lua::new()
    }

    #[test]
    fn test_postgresql_user_requires_name_parameter() {
        let lua = setup_lua();
        let params = lua.create_table().unwrap();

        let result = postgresql_user(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("'name' parameter is required"));
    }

    #[test]
    fn test_postgresql_user_validates_action_parameter() {
        let lua = setup_lua();
        let params = lua.create_table().unwrap();
        params.set("name", "test_user").unwrap();
        params.set("action", "invalid_action").unwrap();

        let result = postgresql_user(&lua, params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid action"));
    }

    #[test]
    fn test_postgresql_user_defaults_to_create_action() {
        let lua = setup_lua();
        let params = lua.create_table().unwrap();
        params.set("name", "test_user").unwrap();

        let result = postgresql_user(&lua, params);
        assert!(result.is_ok());
        let module = result.unwrap();
        let action: String = module
            .get::<Table>("params")
            .unwrap()
            .get("action")
            .unwrap();
        assert_eq!(action, "create");
    }
}
