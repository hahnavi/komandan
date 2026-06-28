use mlua::{Error::RuntimeError, Lua, Table, Value, chunk};

pub fn filter_hosts(lua: &Lua, (hosts, pattern): (Value, Value)) -> mlua::Result<Table> {
    let regex_is_match = lua.create_function(crate::util::regex_is_match)?;
    if hosts.is_nil() {
        return Err(RuntimeError("hosts table must not be nil".to_string()));
    }

    if !hosts.is_table() {
        return Err(RuntimeError("hosts must be a table".to_string()));
    }

    if pattern.is_nil() {
        return Err(RuntimeError("pattern must not be nil".to_string()));
    }

    if !pattern.is_table() && !pattern.is_string() {
        return Err(RuntimeError(
            "pattern must be a string or table".to_string(),
        ));
    }

    let filtered_hosts = lua
        .load(chunk! {
        local hosts = $hosts
        local pattern = $pattern

            if type(pattern) == "string" then
                    -- Treat the single string as a keyword pattern
                    pattern = { pattern }
            end

            local matched_hosts = {}

            for host_key, host_data in pairs(hosts) do
                for _, p in ipairs(pattern) do
                    if type(p) ~= "string" or host_data.name == nil then
                        goto continue
                    end
                    if p:sub(1, 1) ~= "~" then
                        if host_data.name == p then
                            matched_hosts[host_key] = host_data
                            break
                        end
                    else
                        if $regex_is_match(host_data.name, p:sub(2)) then
                            matched_hosts[host_key] = host_data
                            break
                        end
                    end
                    ::continue::
                end

                if host_data.tags ~= nil then
                    for _, tag in ipairs(host_data.tags) do
                        for _, p in ipairs(pattern) do
                            if type(p) ~= "string" then
                                goto continue
                            end
                            if p:sub(1, 1) ~= "~" then
                                if tag == p then
                                    matched_hosts[host_key] = host_data
                                    break
                                end
                            else
                                if $regex_is_match(tag, p:sub(2)) then
                                    matched_hosts[host_key] = host_data
                                    break
                                end
                            end
                            ::continue::
                        end
                    end
                end
            end

            local filtered_hosts = {}
            for _, host_data in pairs(matched_hosts) do
                table.insert(filtered_hosts, host_data)
            end

            return filtered_hosts
            })
        .set_name("filter_hosts")
        .eval::<Table>()?;

    Ok(filtered_hosts)
}
