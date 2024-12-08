use crate::args::Args;
use clap::Parser;
use mlua::{chunk, Lua, Table, Value};

pub fn dprint(lua: &Lua, value: Value) -> mlua::Result<()> {
    let args = Args::parse();
    if args.verbose {
        lua.load(chunk! {
            print($value)
        })
        .exec()?;
    }
    Ok(())
}

pub fn filter_hosts(lua: &Lua, (hosts, pattern): (Table, Value)) -> mlua::Result<Table> {
    let matched_hosts = lua
        .load(chunk! {
        local hosts = $hosts
        local pattern = $pattern

        if hosts == nil then
                error("hosts table must not be nil")
        end

        if type(hosts) ~= "table" then
                error("hosts must be a table")
        end

                    if pattern == nil then
                    error("hosts pattern must not be nil")
                    end

            if type(pattern) == "string" then
                    -- Treat the single string as a keyword pattern
                    pattern = { pattern }
            end

            if type(pattern) ~= "table" then
                error("pattern must be a string or table")
            end

            local matched_hosts = {}

            for host_key, host_data in pairs(hosts) do
                for _, p in ipairs(pattern) do
                    if type(p) ~= "string" then
                        goto continue
                    end
                    if p:sub(1, 1) ~= "~" then
                        if host_key == p then
                            matched_hosts[host_key] = host_data
                            break
                        end
                    else
                        if komandan.regex_is_match(host_key, p:sub(2)) then
                            matched_hosts[host_key] = host_data
                            break
                        end
                    end
                    ::continue::
                end

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
                            if komandan.regex_is_match(tag, p:sub(2)) then
                                matched_hosts[host_key] = host_data
                                break
                            end
                        end
                        ::continue::
                    end
                end
            end

            return matched_hosts
            })
        .set_name("filter_hosts")
        .eval::<Table>()?;

    Ok(matched_hosts)
}

pub fn regex_is_match(
    _: &Lua,
    (text, pattern): (mlua::String, mlua::String),
) -> mlua::Result<bool> {
    let re = match regex::Regex::new(&pattern.to_str()?) {
        Ok(re) => re,
        Err(_) => return Ok(false),
    };
    Ok(re.is_match(&text.to_str()?))
}
