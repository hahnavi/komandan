use crate::util::dprint;
use crate::validator::validate_host;
use http_klien::create_client_from_url;
use mlua::{Error::RuntimeError, Lua, LuaSerdeExt, Table, Value};
use std::{fs::File, io::Read};

pub fn parse_hosts_json_file(lua: &Lua, path: Value) -> mlua::Result<Table> {
    let Value::String(path_lua_str) = path else {
        return Err(RuntimeError(String::from("Path must be a string")));
    };
    let path = path_lua_str.to_str()?.to_owned();

    let Ok(mut file) = File::open(&path) else {
        return Err(RuntimeError(String::from("Failed to open JSON file")));
    };
    let mut content = String::new();

    let hosts = match file.read_to_string(&mut content) {
        Ok(_) => match parse_hosts_json(lua, &content) {
            Ok(h) => h,
            Err(_) => {
                return Err(RuntimeError(format!(
                    "Failed to parse JSON file from '{path}'"
                )));
            }
        },
        Err(_) => return Err(RuntimeError(String::from("Failed to read JSON file"))),
    };

    dprint(
        lua,
        Value::String(lua.create_string(format!(
            "Loaded {} hosts from JSON file '{}'",
            hosts.len()?,
            path
        ))?),
    )?;

    Ok(hosts)
}

pub fn parse_hosts_json_url(lua: &Lua, url: Value) -> mlua::Result<Table> {
    let Value::String(url_lua_str) = url else {
        return Err(RuntimeError(String::from("URL must be a string")));
    };
    let url = url_lua_str.to_str()?.to_owned();

    let (client, path) = create_client_from_url(&url)
        .map_err(|e| RuntimeError(format!("Failed to create client: {e}")))?;

    let content = match client.get(&path) {
        Ok(response) => {
            if !response.is_success() {
                return Err(RuntimeError(format!(
                    "HTTP request failed with status: {}",
                    response.status_code
                )));
            }
            String::from_utf8_lossy(&response.body).to_string()
        }
        Err(e) => {
            return Err(RuntimeError(format!("Failed to fetch URL: {e:?}")));
        }
    };

    let Ok(hosts) = parse_hosts_json(lua, &content) else {
        return Err(RuntimeError(format!("Failed to parse JSON from '{url}'")));
    };

    dprint(
        lua,
        Value::String(lua.create_string(format!(
            "Loaded {} hosts from JSON url '{}'",
            hosts.len()?,
            url
        ))?),
    )?;

    Ok(hosts)
}

fn parse_hosts_json(lua: &Lua, content: &str) -> mlua::Result<Table> {
    let json: serde_json::Value = match serde_json::from_str(content) {
        Ok(j) => j,
        Err(_) => return Err(RuntimeError(String::from("Failed to parse JSON"))),
    };

    let hosts = lua.create_table()?;
    let Ok(lua_value) = lua.to_value(&json) else {
        return Err(RuntimeError(String::from("Failed to convert JSON to Lua")));
    };

    let Some(lua_table) = lua_value.as_table() else {
        return Err(RuntimeError(String::from("JSON does not contain a table")));
    };

    for pair in lua_table.pairs() {
        let (_, value): (Value, Value) = pair?;
        if let Ok(host) = validate_host(lua, value) {
            hosts.set(hosts.len()? + 1, host)?;
        }
    }

    Ok(hosts)
}
