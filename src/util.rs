use crate::args::Args;
use crate::validator::validate_host;
use clap::Parser;
use http_klien::create_client_from_url;
use mlua::{Error::RuntimeError, Lua, LuaSerdeExt, Table, Value, chunk};
use std::{fs::File, io::Read};

pub fn dprint(lua: &Lua, value: Value) -> mlua::Result<()> {
    let args = Args::parse();
    if args.flags.verbose {
        lua.load(chunk! {
            print($value)
        })
        .exec()?;
    }
    Ok(())
}

pub fn filter_hosts(lua: &Lua, (hosts, pattern): (Value, Value)) -> mlua::Result<Table> {
    let regex_is_match = lua.create_function(regex_is_match)?;
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

pub fn regex_is_match(
    _: &Lua,
    (text, pattern): (mlua::String, mlua::String),
) -> mlua::Result<bool> {
    let Ok(re) = regex::Regex::new(&pattern.to_str()?) else {
        return Ok(false);
    };
    Ok(re.is_match(&text.to_str()?))
}

pub fn host_display(host: &Table) -> String {
    let address = host.get::<String>("address").unwrap_or_default();

    match host.get::<String>("name") {
        Ok(name) => format!("{name} ({address})"),
        Err(_) => address,
    }
}

pub fn task_display(task: &Table) -> String {
    let module = task.get::<Table>(1).unwrap_or_else(|_| task.clone());
    task.get::<String>("name")
        .unwrap_or_else(|_| module.get::<String>("name").unwrap_or_default())
}

// Tests
#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::create_lua;

    use super::*;
    use std::{env, fs::write, io::Write};

    #[test]
    fn test_dprint_verbose() -> mlua::Result<()> {
        // Simulate verbose flag being set
        let args = Args {
            main_file: None,
            chunk: None,
            command: None,
            flags: crate::args::Flags {
                dry_run: false,
                no_report: false,
                interactive: false,
                verbose: true,
                unsafe_lua: false,
                version: false,
            },
        };
        unsafe { env::set_var("MOCK_ARGS", format!("{args:?}")) };

        let lua = create_lua()?;
        let value = Value::String(lua.create_string("Test verbose print")?);
        assert!(dprint(&lua, value).is_ok());
        Ok(())
    }

    #[test]
    fn test_dprint_not_verbose() -> mlua::Result<()> {
        // Simulate verbose flag not being set
        let args = Args {
            main_file: None,
            chunk: None,
            command: None,
            flags: crate::args::Flags {
                dry_run: false,
                no_report: false,
                interactive: false,
                verbose: false,
                unsafe_lua: false,
                version: false,
            },
        };
        unsafe { env::set_var("MOCK_ARGS", format!("{args:?}")) };

        let lua = create_lua()?;
        let value = Value::String(lua.create_string("Test non-verbose print")?);
        assert!(dprint(&lua, value).is_ok());
        Ok(())
    }

    #[test]
    fn test_filter_hosts_invalid_hosts_type() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = Value::Nil;
        let pattern = Value::String(lua.create_string("host1")?);
        let result = filter_hosts(&lua, (hosts, pattern));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("hosts table must not be nil"));
        }

        let hosts = lua.create_string("not_a_table")?;
        let pattern = Value::String(lua.create_string("host1")?);
        let result = filter_hosts(&lua, (Value::String(hosts), pattern));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("hosts must be a table"));
        }
        Ok(())
    }

    #[test]
    fn test_filter_hosts_invalid_pattern() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        hosts.set("host1", lua.create_table()?)?;
        let pattern = Value::Nil;
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("pattern must not be nil"));
        }

        let hosts = lua.create_table()?;
        hosts.set("host2", lua.create_table()?)?;
        let pattern = Value::Integer(123);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("pattern must be a string or table"));
        }
        Ok(())
    }

    #[test]
    fn test_filter_hosts_single_string_pattern() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        let host_data = lua.create_table()?;
        host_data.set("name", "host1")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(11, host_data)?;
        let pattern = Value::String(lua.create_string("host1")?);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern))?;
        assert!(result.contains_key(1)?);
        Ok(())
    }

    #[test]
    fn test_filter_hosts_table_pattern() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        let host_data = lua.create_table()?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(3, host_data)?;
        let pattern = Value::Table(lua.create_sequence_from(vec!["host1", "tag2"])?);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern))?;
        assert!(result.contains_key(1)?);
        Ok(())
    }

    #[test]
    fn test_filter_hosts_regex_pattern_host() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        let host_data = lua.create_table()?;
        host_data.set("name", "host1")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(10, host_data)?;
        let pattern = Value::Table(lua.create_sequence_from(vec!["~^host.*$"])?);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern))?;
        assert!(result.contains_key(1)?);
        Ok(())
    }

    #[test]
    fn test_filter_hosts_regex_pattern_tag() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        let host_data = lua.create_table()?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(4, host_data)?;
        let pattern = Value::Table(lua.create_sequence_from(vec!["~^tag.*$"])?);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern))?;
        assert!(result.contains_key(1)?);
        Ok(())
    }

    #[test]
    fn test_filter_hosts_valid_tag() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = lua.create_table()?;
        let host_data = lua.create_table()?;
        host_data.set("address", "192.168.1.1")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(1, host_data)?;

        let host_data = lua.create_table()?;
        host_data.set("address", "192.168.1.2")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag3"])?)?;
        hosts.set(2, host_data)?;

        let host_data = lua.create_table()?;
        host_data.set("address", "192.168.1.3")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1"])?)?;
        hosts.set(3, host_data)?;

        let host_data = lua.create_table()?;
        host_data.set("address", "192.168.1.4")?;
        host_data.set("tags", lua.create_sequence_from(vec!["tag1", "tag2"])?)?;
        hosts.set(4, host_data)?;
        let pattern = Value::Table(lua.create_sequence_from(vec!["~^tag.*$"])?);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern))?;
        assert!(result.contains_key(1)?);
        Ok(())
    }

    #[test]
    fn test_filter_hosts_invalid_hosts() -> mlua::Result<()> {
        let lua = create_lua()?;
        let hosts = Value::String(lua.create_string("not_a_table")?);
        let pattern = Value::String(lua.create_string("host1")?);
        let result = filter_hosts(&lua, (hosts, pattern));
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_regex_is_match_valid_match() -> mlua::Result<()> {
        let lua = create_lua()?;
        let text = lua.create_string("hello world")?;
        let pattern = lua.create_string("hello")?;
        let result = regex_is_match(&lua, (text, pattern))?;
        assert!(result);
        Ok(())
    }

    #[test]
    fn test_regex_is_match_valid_no_match() -> mlua::Result<()> {
        let lua = create_lua()?;
        let text = lua.create_string("hello world")?;
        let pattern = lua.create_string("goodbye")?;
        let result = regex_is_match(&lua, (text, pattern))?;
        assert!(!result);
        Ok(())
    }

    #[test]
    fn test_regex_is_match_invalid_regex() -> mlua::Result<()> {
        let lua = create_lua()?;
        let text = lua.create_string("hello world")?;
        let pattern = lua.create_string("[")?;
        let result = regex_is_match(&lua, (text, pattern))?;
        assert!(!result);
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_file_valid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        let json_content = r#"[
            {
                "name": "test-host",
                "address": "192.168.1.1",
                "tags": ["test", "development"],
                "user": "admin"
            }
        ]"#;
        write(temp_file.path(), json_content).map_err(mlua::Error::external)?;

        let lua_string = lua.create_string(
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        let result = parse_hosts_json_file(&lua, Value::String(lua_string));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_file_invalid_path() -> mlua::Result<()> {
        let lua = create_lua()?;
        let lua_string = Value::String(lua.create_string("/nonexistent/path")?);
        let result = parse_hosts_json_file(&lua, lua_string);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to open JSON file"));
        }
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_file_invalid_file() -> mlua::Result<()> {
        let lua = create_lua()?;
        let temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        temp_file
            .as_file()
            .write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x42])
            .map_err(mlua::Error::external)?;

        let lua_string = lua.create_string(
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        let result = parse_hosts_json_file(&lua, Value::String(lua_string));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to read JSON file"));
        }
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_file_invalid_json() -> mlua::Result<()> {
        let lua = create_lua()?;
        let temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        write(temp_file.path(), "invalid json content").map_err(mlua::Error::external)?;

        let lua_string = lua.create_string(
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        let result = parse_hosts_json_file(&lua, Value::String(lua_string));
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_file_invalid_to_lua_value() -> mlua::Result<()> {
        let lua = create_lua()?;
        let temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        write(temp_file.path(), "true").map_err(mlua::Error::external)?;

        let lua_string = lua.create_string(
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        let result = parse_hosts_json_file(&lua, Value::String(lua_string));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to parse JSON file from"));
        }
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_url_invalid_input_type() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = parse_hosts_json_url(&lua, Value::Nil);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("URL must be a string"));
        }
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_url_not_found() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = parse_hosts_json_url(
            &lua,
            Value::String(lua.create_string("https://komandan.vercel.app/examples/hosts.json")?),
        );
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("HTTP request failed with status"));
        }
        Ok(())
    }

    #[test]
    fn test_parse_hosts_json_url_valid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = parse_hosts_json_url(
            &lua,
            Value::String(lua.create_string("https://komandan.surge.sh/examples/hosts.json")?),
        );
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_hostname_display() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with name
        let host = lua.create_table()?;
        host.set("address", "192.168.1.1")?;
        host.set("name", "test")?;
        assert_eq!(host_display(&host), "test (192.168.1.1)");

        // Test without name
        let host = lua.create_table()?;
        host.set("address", "10.0.0.1")?;
        assert_eq!(host_display(&host), "10.0.0.1");
        Ok(())
    }
}
