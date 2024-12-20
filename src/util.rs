use crate::args::Args;
use crate::validator::validate_host;
use clap::Parser;
use mlua::{chunk, Error::RuntimeError, Lua, LuaSerdeExt, Table, Value};
use reqwest::blocking::get;
use std::{fs::File, io::Read};
use url::Url;

pub fn set_defaults(lua: &Lua, data: Value) -> mlua::Result<()> {
    if !data.is_table() {
        return Err(RuntimeError(
            "Parameter for set_defaults must be a table.".to_string(),
        ));
    }

    let defaults = lua
        .globals()
        .get::<Table>("komandan")?
        .get::<Table>("defaults")?;

    for pair in data.as_table().unwrap().pairs() {
        let (key, value): (String, Value) = pair?;
        defaults.set(key, value.clone())?;
    }

    Ok(())
}

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

    let matched_hosts = lua
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
                    if type(p) ~= "string" then
                        goto continue
                    end
                    if p:sub(1, 1) ~= "~" then
                        if host_key == p then
                            matched_hosts[host_key] = host_data
                            break
                        end
                    else
                        if $regex_is_match(host_key, p:sub(2)) then
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
                            if $regex_is_match(tag, p:sub(2)) then
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

pub fn parse_hosts_json(lua: &Lua, src: Value) -> mlua::Result<Table> {
    let src = match src.as_string_lossy() {
        Some(s) => s,
        None => return Err(RuntimeError(String::from("Invalid src path"))),
    };

    let content = if src.starts_with("http://") || src.starts_with("https://") {
        let url = match Url::parse(&src) {
            Ok(url) => url,
            Err(_) => return Err(RuntimeError(String::from("Invalid URL"))),
        };

        match get(url) {
            Ok(response) => {
                if !response.status().is_success() {
                    return Err(RuntimeError(format!(
                        "HTTP request failed with status: {}",
                        response.status()
                    )));
                }
                match response.text() {
                    Ok(text) => text,
                    Err(_) => {
                        return Err(RuntimeError(String::from("Failed to read response body")))
                    }
                }
            }
            Err(_) => return Err(RuntimeError(String::from("Failed to fetch URL"))),
        }
    } else {
        let mut file = match File::open(&src) {
            Ok(f) => f,
            Err(_) => return Err(RuntimeError(String::from("Failed to open JSON file"))),
        };

        let mut content = String::new();
        match file.read_to_string(&mut content) {
            Ok(_) => content,
            Err(_) => return Err(RuntimeError(String::from("Failed to read JSON file"))),
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(j) => j,
        Err(_) => return Err(RuntimeError(String::from("Failed to parse JSON"))),
    };

    let hosts = lua.create_table()?;

    let lua_value = match lua.to_value(&json) {
        Ok(o) => o,
        Err(_) => return Err(RuntimeError(String::from("Failed to convert JSON to Lua"))),
    };

    let lua_table = match lua_value.as_table() {
        Some(t) => t,
        None => return Err(RuntimeError(String::from("JSON does not contain a table"))),
    };

    for pair in lua_table.pairs() {
        let (_, value): (Value, Value) = pair?;
        match validate_host(&lua, value) {
            Ok(host) => {
                hosts.set(hosts.len()? + 1, host)?;
            }
            Err(_) => {}
        };
    }

    dprint(
        &lua,
        lua.to_value(&format!("Loaded {} hosts from '{}'", hosts.len()?, src))?,
    )?;

    Ok(hosts)
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

pub fn host_display(host: &Table) -> String {
    let address = host.get::<String>("address").unwrap();

    match host.get::<String>("name") {
        Ok(name) => format!("{} ({})", name, address),
        Err(_) => format!("{}", address),
    }
}

pub fn task_display(task: &Table) -> String {
    let module = task.get::<Table>(1).unwrap();
    match task.get::<String>("name") {
        Ok(name) => name,
        Err(_) => module.get::<String>("name").unwrap(),
    }
}

// Tests
#[cfg(test)]
mod tests {
    use mlua::Integer;
    use tempfile::NamedTempFile;

    use crate::setup_lua_env;

    use super::*;
    use std::{env, fs::write, io::Write};

    fn setup_lua() -> Lua {
        let lua = Lua::new();

        // Initialize komandan table and defaults
        lua.globals()
            .set("komandan", lua.create_table().unwrap())
            .unwrap();

        let komandan = lua.globals().get::<Table>("komandan").unwrap();
        komandan
            .set("defaults", lua.create_table().unwrap())
            .unwrap();

        lua
    }

    #[test]
    fn test_dprint_verbose() {
        // Simulate verbose flag being set
        let args = Args {
            main_file: None,
            chunk: None,
            interactive: false,
            verbose: true,
            version: false,
        };
        env::set_var("MOCK_ARGS", format!("{:?}", args));

        let lua = setup_lua();
        let value = Value::String(lua.create_string("Test verbose print").unwrap());
        assert!(dprint(&lua, value).is_ok());
    }

    #[test]
    fn test_dprint_not_verbose() {
        // Simulate verbose flag not being set
        let args = Args {
            main_file: None,
            chunk: None,
            interactive: false,
            verbose: false,
            version: false,
        };
        env::set_var("MOCK_ARGS", format!("{:?}", args));

        let lua = setup_lua();
        let value = Value::String(lua.create_string("Test non-verbose print").unwrap());
        assert!(dprint(&lua, value).is_ok());
    }

    #[test]
    fn test_filter_hosts_invalid_hosts_type() {
        let lua = setup_lua();
        let hosts = Value::Nil;
        let pattern = Value::String(lua.create_string("host1").unwrap());
        let result = filter_hosts(&lua, (hosts, pattern));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("hosts table must not be nil"));

        let hosts = lua.create_string("not_a_table").unwrap();
        let pattern = Value::String(lua.create_string("host1").unwrap());
        let result = filter_hosts(&lua, (Value::String(hosts), pattern));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("hosts must be a table"));
    }

    #[test]
    fn test_filter_hosts_invalid_pattern() {
        let lua = setup_lua();
        let hosts = lua.create_table().unwrap();
        hosts.set("host1", lua.create_table().unwrap()).unwrap();
        let pattern = Value::Nil;
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("pattern must not be nil"));

        let hosts = lua.create_table().unwrap();
        hosts.set("host2", lua.create_table().unwrap()).unwrap();
        let pattern = Value::Integer(123);
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("pattern must be a string or table"));
    }

    #[test]
    fn test_filter_hosts_single_string_pattern() {
        let lua = setup_lua();
        let hosts = lua.create_table().unwrap();
        let host_data = lua.create_table().unwrap();
        host_data
            .set(
                "tags",
                lua.create_sequence_from(vec!["tag1", "tag2"]).unwrap(),
            )
            .unwrap();
        hosts.set("host1", host_data).unwrap();
        let pattern = Value::String(lua.create_string("host1").unwrap());
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern)).unwrap();
        assert!(result.contains_key("host1").unwrap());
    }

    #[test]
    fn test_filter_hosts_table_pattern() {
        let lua = setup_lua();
        let hosts = lua.create_table().unwrap();
        let host_data = lua.create_table().unwrap();
        host_data
            .set(
                "tags",
                lua.create_sequence_from(vec!["tag1", "tag2"]).unwrap(),
            )
            .unwrap();
        hosts.set("host1", host_data).unwrap();
        let pattern = Value::Table(lua.create_sequence_from(vec!["host1", "tag2"]).unwrap());
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern)).unwrap();
        assert!(result.contains_key("host1").unwrap());
    }

    #[test]
    fn test_filter_hosts_regex_pattern_host() {
        let lua = setup_lua();
        let hosts = lua.create_table().unwrap();
        let host_data = lua.create_table().unwrap();
        host_data
            .set(
                "tags",
                lua.create_sequence_from(vec!["tag1", "tag2"]).unwrap(),
            )
            .unwrap();
        hosts.set("host1", host_data).unwrap();
        let pattern = Value::Table(lua.create_sequence_from(vec!["~^host.*$"]).unwrap());
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern)).unwrap();
        assert!(result.contains_key("host1").unwrap());
    }

    #[test]
    fn test_filter_hosts_regex_pattern_tag() {
        let lua = setup_lua();
        let hosts = lua.create_table().unwrap();
        let host_data = lua.create_table().unwrap();
        host_data
            .set(
                "tags",
                lua.create_sequence_from(vec!["tag1", "tag2"]).unwrap(),
            )
            .unwrap();
        hosts.set("host1", host_data).unwrap();
        let pattern = Value::Table(lua.create_sequence_from(vec!["~^tag.*$"]).unwrap());
        let result = filter_hosts(&lua, (Value::Table(hosts), pattern)).unwrap();
        assert!(result.contains_key("host1").unwrap());
    }

    #[test]
    fn test_filter_hosts_invalid_hosts() {
        let lua = setup_lua();
        let hosts = Value::String(lua.create_string("not_a_table").unwrap());
        let pattern = Value::String(lua.create_string("host1").unwrap());
        let result = filter_hosts(&lua, (hosts, pattern));
        assert!(result.is_err());
    }

    #[test]
    fn test_regex_is_match_valid_match() {
        let lua = setup_lua();
        let text = lua.create_string("hello world").unwrap();
        let pattern = lua.create_string("hello").unwrap();
        let result = regex_is_match(&lua, (text, pattern)).unwrap();
        assert!(result);
    }

    #[test]
    fn test_regex_is_match_valid_no_match() {
        let lua = setup_lua();
        let text = lua.create_string("hello world").unwrap();
        let pattern = lua.create_string("goodbye").unwrap();
        let result = regex_is_match(&lua, (text, pattern)).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_regex_is_match_invalid_regex() {
        let lua = setup_lua();
        let text = lua.create_string("hello world").unwrap();
        let pattern = lua.create_string("[").unwrap();
        let result = regex_is_match(&lua, (text, pattern)).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_set_defaults() {
        let lua = Lua::new();
        setup_lua_env(&lua).unwrap();

        // Test setting a default value
        let defaults_data = lua.create_table().unwrap();
        defaults_data.set("user", "testuser").unwrap();
        set_defaults(&lua, Value::Table(defaults_data)).unwrap();

        let defaults = lua
            .globals()
            .get::<Table>("komandan")
            .unwrap()
            .get::<Table>("defaults")
            .unwrap();
        assert_eq!(defaults.get::<String>("user").unwrap(), "testuser");

        // Test setting multiple default values
        let defaults_data = lua.create_table().unwrap();
        defaults_data.set("port", 2222).unwrap();
        defaults_data.set("key", "/path/to/key").unwrap();
        set_defaults(&lua, Value::Table(defaults_data)).unwrap();

        let defaults = lua
            .globals()
            .get::<Table>("komandan")
            .unwrap()
            .get::<Table>("defaults")
            .unwrap();
        assert_eq!(defaults.get::<Integer>("port").unwrap(), 2222);
        assert_eq!(defaults.get::<String>("key").unwrap(), "/path/to/key");

        // Test with non-table input
        let result = set_defaults(
            &lua,
            Value::String(lua.create_string("not_a_table").unwrap()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_hosts_json_valid() {
        let lua = setup_lua();
        let temp_file = NamedTempFile::new().unwrap();
        let json_content = r#"[
            {
                "name": "test-host",
                "address": "192.168.1.1",
                "tags": ["test", "development"],
                "user": "admin"
            }
        ]"#;
        write(temp_file.path(), json_content).unwrap();

        let lua_string = lua
            .create_string(temp_file.path().to_str().unwrap())
            .unwrap();
        let result = parse_hosts_json(&lua, Value::String(lua_string));
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_hosts_json_invalid_path() {
        let lua = setup_lua();
        let lua_string = Value::String(lua.create_string("/nonexistent/path").unwrap());
        let result = parse_hosts_json(&lua, lua_string);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to open JSON file"));
    }

    #[test]
    fn test_parse_hosts_json_invalid_file() {
        let lua = setup_lua();
        let temp_file = NamedTempFile::new().unwrap();
        temp_file
            .as_file()
            .write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x42])
            .unwrap();

        let lua_string = lua
            .create_string(temp_file.path().to_str().unwrap())
            .unwrap();
        let result = parse_hosts_json(&lua, Value::String(lua_string));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to read JSON file"));
    }

    #[test]
    fn test_parse_hosts_json_invalid_json() {
        let lua = setup_lua();
        let temp_file = NamedTempFile::new().unwrap();
        write(temp_file.path(), "invalid json content").unwrap();

        let lua_string = lua
            .create_string(temp_file.path().to_str().unwrap())
            .unwrap();
        let result = parse_hosts_json(&lua, Value::String(lua_string));
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_hosts_json_invalid_to_lua_value() {
        let lua = setup_lua();
        let temp_file = NamedTempFile::new().unwrap();
        write(temp_file.path(), "true").unwrap();

        let lua_string = lua
            .create_string(temp_file.path().to_str().unwrap())
            .unwrap();
        let result = parse_hosts_json(&lua, Value::String(lua_string));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("JSON does not contain a table"));
    }

    #[test]
    fn test_parse_hosts_json_invalid_input_type() {
        let lua = setup_lua();
        let result = parse_hosts_json(&lua, Value::Nil);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid src path"));
    }

    #[test]
    fn test_hostname_display() {
        let lua = Lua::new();

        // Test with name
        let host = lua.create_table().unwrap();
        host.set("address", "192.168.1.1").unwrap();
        host.set("name", "test").unwrap();
        assert_eq!(host_display(&host), "test (192.168.1.1)");

        // Test without name
        let host = lua.create_table().unwrap();
        host.set("address", "10.0.0.1").unwrap();
        assert_eq!(host_display(&host), "10.0.0.1");
    }
}
