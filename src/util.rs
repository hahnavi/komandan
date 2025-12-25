use crate::args::Args;
use crate::executor::CommandExecutor;
use crate::local::LocalSession;
use crate::models::ConnectionType;
use crate::ssh::{SSHAuthMethod, SSHSession};
use crate::validator::validate_host;
use anyhow::{Context, Result};
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

#[derive(Debug, Default)]
struct HostInfo {
    os: OSInfo,
    cpu: CPUInfo,
    memory: MemoryInfo,
}

#[derive(Debug, Default)]
struct OSInfo {
    distribution: Option<String>,
    version: Option<String>,
    family: Option<String>,
    kernel: Option<String>,
    hostname: Option<String>,
}

#[derive(Debug, Default)]
struct CPUInfo {
    model: Option<String>,
    cores: Option<u32>,
}

#[derive(Debug, Default)]
struct MemoryInfo {
    total_mb: Option<u64>,
    free_mb: Option<u64>,
}

/// Gathers system information from a remote host
pub fn host_info(lua: &Lua, host: Value) -> mlua::Result<Table> {
    // Input validation using existing validate_host function
    let host_table = validate_host(lua, host)
        .map_err(|e| mlua::Error::RuntimeError(format!("Host validation failed: {e}")))?;

    // Additional validation for required authentication fields
    validate_host_authentication(&host_table)?;

    // Create connection and gather info with graceful error handling
    let info = create_connection_and_gather_info(&host_table);

    // Convert to Lua table - always returns a complete table structure
    create_info_table(lua, info)
}

/// Validates host authentication configuration
fn validate_host_authentication(host_table: &Table) -> mlua::Result<()> {
    let connection_type = get_connection_type(host_table);

    match connection_type {
        ConnectionType::Local => {
            // Local connections don't need authentication validation
            Ok(())
        }
        ConnectionType::SSH => {
            // For SSH connections, ensure we have either password or private key
            let has_password = host_table.get::<String>("password").is_ok();
            let has_private_key = host_table.get::<String>("private_key_file").is_ok();

            if !has_password && !has_private_key {
                return Err(mlua::Error::RuntimeError(
                    "SSH connection requires either 'password' or 'private_key_file' parameter"
                        .to_string(),
                ));
            }

            // Validate username is provided for SSH
            if host_table.get::<String>("user").is_err() {
                return Err(mlua::Error::RuntimeError(
                    "SSH connection requires 'user' parameter".to_string(),
                ));
            }

            Ok(())
        }
    }
}

/// Creates a `HostInfo` structure with all fields set to "Unknown" values
/// Used when connection or script execution fails
fn create_unknown_host_info() -> HostInfo {
    HostInfo {
        os: OSInfo {
            distribution: Some("Unknown".to_string()),
            version: Some("Unknown".to_string()),
            family: Some("unknown".to_string()),
            kernel: Some("Unknown".to_string()),
            hostname: Some("Unknown".to_string()),
        },
        cpu: CPUInfo {
            model: Some("Unknown".to_string()),
            cores: Some(0),
        },
        memory: MemoryInfo {
            total_mb: Some(0),
            free_mb: Some(0),
        },
    }
}

fn create_connection_and_gather_info(host_table: &Table) -> HostInfo {
    let script = r#"
{
  # Distribution and version
  if command -v lsb_release >/dev/null 2>&1; then
    echo "DISTRO=$(lsb_release -si 2>/dev/null || echo 'Unknown')"
    echo "VERSION=$(lsb_release -sr 2>/dev/null || echo 'Unknown')"
    echo "FAMILY=$(lsb_release -si 2>/dev/null | tr '[:upper:]' '[:lower:]' || echo 'unknown')"
  elif [ -f /etc/os-release ]; then
    echo "DISTRO=$(grep '^NAME=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' || echo 'Unknown')"
    echo "VERSION=$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' || echo 'Unknown')"
    echo "FAMILY=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' || echo 'unknown')"
  elif [ -f /etc/redhat-release ]; then
    echo "DISTRO=$(cat /etc/redhat-release | cut -d' ' -f1 || echo 'Unknown')"
    echo "VERSION=$(cat /etc/redhat-release | grep -o '[0-9]\+\.[0-9]\+' | head -1 || echo 'Unknown')"
    echo "FAMILY=rhel"
  else
    echo "DISTRO=Unknown"
    echo "VERSION=Unknown"
    echo "FAMILY=unknown"
  fi

  # Kernel and hostname with fallbacks
  echo "KERNEL=$(uname -r 2>/dev/null || echo 'Unknown')"
  echo "HOSTNAME=$(hostname 2>/dev/null || hostnamectl --static 2>/dev/null || cat /etc/hostname 2>/dev/null || echo 'Unknown')"

  # CPU info
  echo "CPU_MODEL=$(grep '^model name' /proc/cpuinfo | head -1 | cut -d':' -f2 | sed 's/^ *//' 2>/dev/null || echo 'Unknown')"
  echo "CPU_CORES=$(nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null || echo '0')"

  # Memory info
  if [ -f /proc/meminfo ]; then
    echo "MEM_TOTAL_KB=$(grep '^MemTotal:' /proc/meminfo | awk '{print $2}' 2>/dev/null || echo '0')"
    echo "MEM_AVAILABLE_KB=$(grep '^MemAvailable:' /proc/meminfo | awk '{print $2}' 2>/dev/null || grep '^MemFree:' /proc/meminfo | awk '{print $2}' 2>/dev/null || echo '0')"
  else
    echo "MEM_TOTAL_KB=0"
    echo "MEM_AVAILABLE_KB=0"
  fi
} 2>/dev/null
"#;

    let (stdout, _stderr, exit_code) = match get_connection_type(host_table) {
        ConnectionType::Local => {
            let mut local_session = LocalSession::new();
            match local_session.cmd(script) {
                Ok(result) => result,
                Err(_) => return create_unknown_host_info(),
            }
        }
        ConnectionType::SSH => {
            let Ok(ssh_session) = create_ssh_connection(host_table) else {
                return create_unknown_host_info();
            };
            let mut ssh_session = ssh_session;
            match ssh_session.cmd(script) {
                Ok(result) => result,
                Err(_) => return create_unknown_host_info(),
            }
        }
    };

    // Handle script execution failures gracefully
    if exit_code != 0 {
        return create_unknown_host_info();
    }

    // Parse script output into structured data
    parse_host_info_output(&stdout)
}

fn get_connection_type(host_table: &Table) -> ConnectionType {
    match host_table.get::<String>("connection") {
        Ok(conn_str) if conn_str.to_lowercase() == "local" => ConnectionType::Local,
        _ => ConnectionType::SSH, // Default to SSH
    }
}

fn create_ssh_connection(host_table: &Table) -> Result<SSHSession> {
    let mut ssh_session = SSHSession::new().context("Failed to initialize SSH session")?;

    // Extract connection parameters with validation
    let address = host_table
        .get::<String>("address")
        .map_err(|_| anyhow::anyhow!("Host address is required for SSH connection"))?;

    let port = host_table.get::<u16>("port").unwrap_or(22);

    let username = host_table
        .get::<String>("user")
        .map_err(|_| anyhow::anyhow!("Username is required for SSH connection"))?;

    // Set up authentication method with detailed error handling
    let auth_method = if let Ok(password) = host_table.get::<String>("password") {
        SSHAuthMethod::Password(password)
    } else if let Ok(private_key_file) = host_table.get::<String>("private_key_file") {
        let passphrase = host_table.get::<String>("private_key_pass").ok();

        // Validate private key file exists (basic check)
        if private_key_file.is_empty() {
            return Err(anyhow::anyhow!("Private key file path cannot be empty"));
        }

        SSHAuthMethod::PublicKey {
            private_key: private_key_file,
            passphrase,
        }
    } else {
        return Err(anyhow::anyhow!(
            "No valid authentication method provided. SSH requires either 'password' or 'private_key_file'"
        ));
    };

    // Connect to SSH server with detailed error context
    ssh_session
        .connect(&address, port, &username, auth_method)
        .with_context(|| {
            format!("Failed to establish SSH connection to {username}@{address}:{port}")
        })?;

    Ok(ssh_session)
}

fn parse_host_info_output(output: &str) -> HostInfo {
    let mut os_info = OSInfo::default();
    let mut cpu_info = CPUInfo::default();
    let mut memory_info = MemoryInfo::default();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = if value == "Unknown" || value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };

            match key {
                "DISTRO" => os_info.distribution = value,
                "VERSION" => os_info.version = value,
                "FAMILY" => os_info.family = value,
                "KERNEL" => os_info.kernel = value,
                "HOSTNAME" => os_info.hostname = value,
                "CPU_MODEL" => cpu_info.model = value,
                "CPU_CORES" => {
                    if let Some(cores_str) = value {
                        cpu_info.cores = cores_str.parse().ok();
                    }
                }
                "MEM_TOTAL_KB" => {
                    if let Some(kb_str) = value
                        && let Ok(kb) = kb_str.parse::<u64>()
                    {
                        memory_info.total_mb = Some(kb / 1024);
                    }
                }
                "MEM_AVAILABLE_KB" => {
                    if let Some(kb_str) = value
                        && let Ok(kb) = kb_str.parse::<u64>()
                    {
                        memory_info.free_mb = Some(kb / 1024);
                    }
                }
                _ => {} // Ignore unknown keys
            }
        }
    }

    HostInfo {
        os: os_info,
        cpu: cpu_info,
        memory: memory_info,
    }
}

fn create_info_table(lua: &Lua, info: HostInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    // Create OS section
    let os_table = lua.create_table()?;
    if let Some(distribution) = info.os.distribution {
        os_table.set("distribution", distribution)?;
    }
    if let Some(version) = info.os.version {
        os_table.set("version", version)?;
    }
    if let Some(family) = info.os.family {
        os_table.set("family", family)?;
    }
    if let Some(kernel) = info.os.kernel {
        os_table.set("kernel", kernel)?;
    }
    if let Some(hostname) = info.os.hostname {
        os_table.set("hostname", hostname)?;
    }
    table.set("os", os_table)?;

    // Create CPU section
    let cpu_table = lua.create_table()?;
    if let Some(model) = info.cpu.model {
        cpu_table.set("model", model)?;
    }
    if let Some(cores) = info.cpu.cores {
        cpu_table.set("cores", cores)?;
    }
    table.set("cpu", cpu_table)?;

    // Create memory section
    let memory_table = lua.create_table()?;
    if let Some(total_mb) = info.memory.total_mb {
        memory_table.set("total_mb", total_mb)?;
    }
    if let Some(free_mb) = info.memory.free_mb {
        memory_table.set("free_mb", free_mb)?;
    }
    table.set("memory", memory_table)?;

    Ok(table)
}

// Tests
#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use crate::create_lua;

    use super::*;
    use std::{env, fs::write, io::Write};

    #[test]
    #[allow(unsafe_code)]
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
    #[allow(unsafe_code)]
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
            Value::String(lua.create_string("https://komandan.vercel.app/examples/hosts2.json")?),
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
            Value::String(lua.create_string("https://komandan.vercel.app/examples/hosts.json")?),
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

    #[test]
    fn test_host_info_local_connection() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Create a local host configuration
        let host = lua.create_table()?;
        host.set("address", "localhost")?;
        host.set("connection", "local")?;

        // Call host_info function
        let result = host_info(&lua, Value::Table(host))?;

        // Verify the structure of the returned table
        assert!(result.contains_key("os")?);
        assert!(result.contains_key("cpu")?);
        assert!(result.contains_key("memory")?);

        // Check OS section exists (fields may be nil if not determinable)
        let _os_table = result.get::<Table>("os")?;
        // Just verify the table exists - fields may be nil

        // Check CPU section exists (fields may be nil if not determinable)
        let _cpu_table = result.get::<Table>("cpu")?;
        // Just verify the table exists - fields may be nil

        // Check memory section exists (fields may be nil if not determinable)
        let _memory_table = result.get::<Table>("memory")?;
        // Just verify the table exists - fields may be nil

        Ok(())
    }

    #[test]
    fn test_host_info_invalid_host() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with invalid host (missing address)
        let host = lua.create_table()?;
        host.set("connection", "local")?;

        let result = host_info(&lua, Value::Table(host));
        assert!(result.is_err());

        // Check that the error message is descriptive
        if let Err(e) = result {
            assert!(e.to_string().contains("Host validation failed"));
        }

        Ok(())
    }

    #[test]
    fn test_host_info_ssh_missing_auth() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test SSH host without authentication method
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("user", "testuser")?;
        // No password or private_key_file

        let result = host_info(&lua, Value::Table(host));
        assert!(result.is_err());

        // Check that the error message mentions authentication
        if let Err(e) = result {
            let error_msg = e.to_string();
            eprintln!("Actual error message: {}", error_msg);
            assert!(error_msg.contains("authentication") || error_msg.contains("SSH"));
        }

        Ok(())
    }

    #[test]
    fn test_host_info_ssh_missing_user() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test SSH host without user
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("password", "testpass")?;
        // No user

        let result = host_info(&lua, Value::Table(host));
        assert!(result.is_err());

        // Check that the error message mentions user requirement
        if let Err(e) = result {
            assert!(e.to_string().contains("user"));
        }

        Ok(())
    }

    #[test]
    fn test_host_info_connection_failure_returns_unknown() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with SSH host that will fail to connect (invalid address)
        let host = lua.create_table()?;
        host.set("address", "invalid.nonexistent.host.example")?;
        host.set("user", "testuser")?;
        host.set("password", "testpass")?;

        // This should not error but return a table with "Unknown" values
        let result = host_info(&lua, Value::Table(host))?;

        // Verify the structure exists
        assert!(result.contains_key("os")?);
        assert!(result.contains_key("cpu")?);
        assert!(result.contains_key("memory")?);

        // Check that OS fields contain "Unknown" values
        let os_table = result.get::<Table>("os")?;
        if let Ok(distribution) = os_table.get::<String>("distribution") {
            assert_eq!(distribution, "Unknown");
        }

        Ok(())
    }

    #[test]
    fn test_validate_host_authentication_local() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "localhost")?;
        host.set("connection", "local")?;

        // Local connections should not require authentication
        let result = validate_host_authentication(&host);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_validate_host_authentication_ssh_with_password() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("user", "testuser")?;
        host.set("password", "testpass")?;

        // SSH with password should be valid
        let result = validate_host_authentication(&host);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_validate_host_authentication_ssh_with_key() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("user", "testuser")?;
        host.set("private_key_file", "/path/to/key")?;

        // SSH with private key should be valid
        let result = validate_host_authentication(&host);
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn test_validate_host_authentication_ssh_no_auth() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("user", "testuser")?;
        // No password or private_key_file

        // SSH without authentication should fail
        let result = validate_host_authentication(&host);
        assert!(result.is_err());

        if let Err(e) = result {
            let error_msg = e.to_string();
            eprintln!("Actual error message: {}", error_msg);
            assert!(
                error_msg.contains("authentication")
                    || error_msg.contains("SSH")
                    || error_msg.contains("password")
                    || error_msg.contains("private_key")
            );
        }

        Ok(())
    }

    #[test]
    fn test_validate_host_authentication_ssh_no_user() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("password", "testpass")?;
        // No user

        // SSH without user should fail
        let result = validate_host_authentication(&host);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e.to_string().contains("user"));
        }

        Ok(())
    }

    #[test]
    fn test_create_unknown_host_info() {
        let info = create_unknown_host_info();

        // Check OS info has "Unknown" values
        assert_eq!(info.os.distribution, Some("Unknown".to_string()));
        assert_eq!(info.os.version, Some("Unknown".to_string()));
        assert_eq!(info.os.family, Some("unknown".to_string()));
        assert_eq!(info.os.kernel, Some("Unknown".to_string()));
        assert_eq!(info.os.hostname, Some("Unknown".to_string()));

        // Check CPU info has "Unknown" values
        assert_eq!(info.cpu.model, Some("Unknown".to_string()));
        assert_eq!(info.cpu.cores, Some(0));

        // Check memory info has zero values
        assert_eq!(info.memory.total_mb, Some(0));
        assert_eq!(info.memory.free_mb, Some(0));
    }

    #[test]
    fn test_parse_host_info_output() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=test-host
CPU_MODEL=Intel(R) Xeon(R) CPU E5-2676 v3 @ 2.40GHz
CPU_CORES=2
MEM_TOTAL_KB=4026368
MEM_AVAILABLE_KB=461824"#;

        let info = parse_host_info_output(sample_output);

        // Check OS info
        assert_eq!(info.os.distribution, Some("Ubuntu".to_string()));
        assert_eq!(info.os.version, Some("22.04".to_string()));
        assert_eq!(info.os.family, Some("ubuntu".to_string()));
        assert_eq!(info.os.kernel, Some("5.15.0-101-generic".to_string()));
        assert_eq!(info.os.hostname, Some("test-host".to_string()));

        // Check CPU info
        assert_eq!(
            info.cpu.model,
            Some("Intel(R) Xeon(R) CPU E5-2676 v3 @ 2.40GHz".to_string())
        );
        assert_eq!(info.cpu.cores, Some(2));

        // Check memory info (converted from KB to MB)
        assert_eq!(info.memory.total_mb, Some(3932)); // 4026368 / 1024
        assert_eq!(info.memory.free_mb, Some(451)); // 461824 / 1024

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_with_unknown_values() -> Result<()> {
        let sample_output = r#"DISTRO=Unknown
VERSION=Unknown
FAMILY=Unknown
KERNEL=Unknown
HOSTNAME=Unknown
CPU_MODEL=Unknown
CPU_CORES=0
MEM_TOTAL_KB=0
MEM_AVAILABLE_KB=0"#;

        let info = parse_host_info_output(sample_output);

        // Check that "Unknown" strings are converted to None
        assert_eq!(info.os.distribution, None);
        assert_eq!(info.os.version, None);
        assert_eq!(info.os.family, None);
        assert_eq!(info.os.kernel, None);
        assert_eq!(info.os.hostname, None);
        assert_eq!(info.cpu.model, None);

        // Check that numeric values are still parsed
        assert_eq!(info.cpu.cores, Some(0));
        assert_eq!(info.memory.total_mb, Some(0));
        assert_eq!(info.memory.free_mb, Some(0));

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_partial_data() -> Result<()> {
        let sample_output = r#"DISTRO=CentOS
VERSION=8.5
FAMILY=rhel
CPU_CORES=4"#;

        let info = parse_host_info_output(sample_output);

        // Check that present values are parsed
        assert_eq!(info.os.distribution, Some("CentOS".to_string()));
        assert_eq!(info.os.version, Some("8.5".to_string()));
        assert_eq!(info.os.family, Some("rhel".to_string()));
        assert_eq!(info.cpu.cores, Some(4));

        // Check that missing values are None
        assert_eq!(info.os.kernel, None);
        assert_eq!(info.os.hostname, None);
        assert_eq!(info.cpu.model, None);
        assert_eq!(info.memory.total_mb, None);
        assert_eq!(info.memory.free_mb, None);

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_lowercase_unknown() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
FAMILY=unknown
KERNEL=5.15.0-101-generic"#;

        let info = parse_host_info_output(sample_output);

        // Check that "unknown" (lowercase) is treated as a valid value
        assert_eq!(info.os.distribution, Some("Ubuntu".to_string()));
        assert_eq!(info.os.family, Some("unknown".to_string())); // lowercase "unknown" is kept
        assert_eq!(info.os.kernel, Some("5.15.0-101-generic".to_string()));

        Ok(())
    }

    // Additional comprehensive tests for shell script parsing with various mock outputs

    #[test]
    fn test_parse_host_info_output_debian_system() -> Result<()> {
        let sample_output = r#"DISTRO=Debian GNU/Linux
VERSION=11
FAMILY=debian
KERNEL=5.10.0-21-amd64
HOSTNAME=debian-server
CPU_MODEL=AMD EPYC 7763 64-Core Processor
CPU_CORES=8
MEM_TOTAL_KB=8388608
MEM_AVAILABLE_KB=6291456"#;

        let info = parse_host_info_output(sample_output);

        assert_eq!(info.os.distribution, Some("Debian GNU/Linux".to_string()));
        assert_eq!(info.os.version, Some("11".to_string()));
        assert_eq!(info.os.family, Some("debian".to_string()));
        assert_eq!(info.os.kernel, Some("5.10.0-21-amd64".to_string()));
        assert_eq!(info.os.hostname, Some("debian-server".to_string()));
        assert_eq!(
            info.cpu.model,
            Some("AMD EPYC 7763 64-Core Processor".to_string())
        );
        assert_eq!(info.cpu.cores, Some(8));
        assert_eq!(info.memory.total_mb, Some(8192)); // 8388608 / 1024
        assert_eq!(info.memory.free_mb, Some(6144)); // 6291456 / 1024

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_rhel_system() -> Result<()> {
        let sample_output = r#"DISTRO=Red Hat Enterprise Linux
VERSION=8.6
FAMILY=rhel
KERNEL=4.18.0-372.9.1.el8.x86_64
HOSTNAME=rhel-production
CPU_MODEL=Intel(R) Xeon(R) Gold 6248 CPU @ 2.50GHz
CPU_CORES=16
MEM_TOTAL_KB=16777216
MEM_AVAILABLE_KB=12582912"#;

        let info = parse_host_info_output(sample_output);

        assert_eq!(
            info.os.distribution,
            Some("Red Hat Enterprise Linux".to_string())
        );
        assert_eq!(info.os.version, Some("8.6".to_string()));
        assert_eq!(info.os.family, Some("rhel".to_string()));
        assert_eq!(
            info.os.kernel,
            Some("4.18.0-372.9.1.el8.x86_64".to_string())
        );
        assert_eq!(info.os.hostname, Some("rhel-production".to_string()));
        assert_eq!(
            info.cpu.model,
            Some("Intel(R) Xeon(R) Gold 6248 CPU @ 2.50GHz".to_string())
        );
        assert_eq!(info.cpu.cores, Some(16));
        assert_eq!(info.memory.total_mb, Some(16384)); // 16777216 / 1024
        assert_eq!(info.memory.free_mb, Some(12288)); // 12582912 / 1024

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_malformed_lines() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
INVALID_LINE_WITHOUT_EQUALS
FAMILY=ubuntu
=EMPTY_KEY
KERNEL=
CPU_CORES=not_a_number
MEM_TOTAL_KB=abc123"#;

        let info = parse_host_info_output(sample_output);

        // Valid lines should be parsed correctly
        assert_eq!(info.os.distribution, Some("Ubuntu".to_string()));
        assert_eq!(info.os.version, Some("22.04".to_string()));
        assert_eq!(info.os.family, Some("ubuntu".to_string()));

        // Empty values should be treated as None
        assert_eq!(info.os.kernel, None);

        // Invalid numeric values should be None
        assert_eq!(info.cpu.cores, None);
        assert_eq!(info.memory.total_mb, None);

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_empty_input() -> Result<()> {
        let sample_output = "";
        let info = parse_host_info_output(sample_output);

        // All fields should be None for empty input
        assert_eq!(info.os.distribution, None);
        assert_eq!(info.os.version, None);
        assert_eq!(info.os.family, None);
        assert_eq!(info.os.kernel, None);
        assert_eq!(info.os.hostname, None);
        assert_eq!(info.cpu.model, None);
        assert_eq!(info.cpu.cores, None);
        assert_eq!(info.memory.total_mb, None);
        assert_eq!(info.memory.free_mb, None);

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_whitespace_values() -> Result<()> {
        let sample_output = r#"DISTRO=   Ubuntu
VERSION=  22.04
FAMILY=ubuntu
KERNEL=
HOSTNAME=    test-host
CPU_MODEL=  Intel CPU
CPU_CORES=4
MEM_TOTAL_KB=1048576"#;

        let info = parse_host_info_output(sample_output);

        // Values with leading/trailing whitespace should be preserved as-is
        // Note: The parsing logic processes lines as they are
        assert_eq!(info.os.distribution, Some("   Ubuntu".to_string()));
        assert_eq!(info.os.version, Some("  22.04".to_string()));
        assert_eq!(info.os.family, Some("ubuntu".to_string()));
        assert_eq!(info.os.kernel, None); // Empty after trimming
        assert_eq!(info.os.hostname, Some("    test-host".to_string()));
        assert_eq!(info.cpu.model, Some("  Intel CPU".to_string()));

        // For numeric values, clean values should parse correctly
        assert_eq!(info.cpu.cores, Some(4));
        assert_eq!(info.memory.total_mb, Some(1024)); // 1048576 / 1024

        Ok(())
    }

    // Tests for connection type determination

    #[test]
    fn test_get_connection_type_local() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "localhost")?;
        host.set("connection", "local")?;

        let conn_type = get_connection_type(&host);
        assert_eq!(conn_type, ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_get_connection_type_local_case_insensitive() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "localhost")?;
        host.set("connection", "LOCAL")?;

        let conn_type = get_connection_type(&host);
        assert_eq!(conn_type, ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_get_connection_type_ssh_default() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        // No connection field - should default to SSH

        let conn_type = get_connection_type(&host);
        assert_eq!(conn_type, ConnectionType::SSH);

        Ok(())
    }

    #[test]
    fn test_get_connection_type_ssh_explicit() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("connection", "ssh")?;

        let conn_type = get_connection_type(&host);
        assert_eq!(conn_type, ConnectionType::SSH);

        Ok(())
    }

    // Tests for create_info_table function

    #[test]
    fn test_create_info_table_complete_data() -> mlua::Result<()> {
        let lua = create_lua()?;
        let info = HostInfo {
            os: OSInfo {
                distribution: Some("Ubuntu".to_string()),
                version: Some("22.04".to_string()),
                family: Some("ubuntu".to_string()),
                kernel: Some("5.15.0-101-generic".to_string()),
                hostname: Some("test-host".to_string()),
            },
            cpu: CPUInfo {
                model: Some("Intel CPU".to_string()),
                cores: Some(4),
            },
            memory: MemoryInfo {
                total_mb: Some(8192),
                free_mb: Some(4096),
            },
        };

        let table = create_info_table(&lua, info)?;

        // Check OS section
        let os_table = table.get::<Table>("os")?;
        assert_eq!(os_table.get::<String>("distribution")?, "Ubuntu");
        assert_eq!(os_table.get::<String>("version")?, "22.04");
        assert_eq!(os_table.get::<String>("family")?, "ubuntu");
        assert_eq!(os_table.get::<String>("kernel")?, "5.15.0-101-generic");
        assert_eq!(os_table.get::<String>("hostname")?, "test-host");

        // Check CPU section
        let cpu_table = table.get::<Table>("cpu")?;
        assert_eq!(cpu_table.get::<String>("model")?, "Intel CPU");
        assert_eq!(cpu_table.get::<u32>("cores")?, 4);

        // Check memory section
        let memory_table = table.get::<Table>("memory")?;
        assert_eq!(memory_table.get::<u64>("total_mb")?, 8192);
        assert_eq!(memory_table.get::<u64>("free_mb")?, 4096);

        Ok(())
    }

    #[test]
    fn test_create_info_table_partial_data() -> mlua::Result<()> {
        let lua = create_lua()?;
        let info = HostInfo {
            os: OSInfo {
                distribution: Some("CentOS".to_string()),
                version: None, // Missing version
                family: Some("rhel".to_string()),
                kernel: None, // Missing kernel
                hostname: Some("centos-server".to_string()),
            },
            cpu: CPUInfo {
                model: None, // Missing model
                cores: Some(2),
            },
            memory: MemoryInfo {
                total_mb: Some(4096),
                free_mb: None, // Missing free memory
            },
        };

        let table = create_info_table(&lua, info)?;

        // Check OS section - only present fields should exist
        let os_table = table.get::<Table>("os")?;
        assert_eq!(os_table.get::<String>("distribution")?, "CentOS");
        assert!(os_table.get::<String>("version").is_err()); // Should not exist
        assert_eq!(os_table.get::<String>("family")?, "rhel");
        assert!(os_table.get::<String>("kernel").is_err()); // Should not exist
        assert_eq!(os_table.get::<String>("hostname")?, "centos-server");

        // Check CPU section
        let cpu_table = table.get::<Table>("cpu")?;
        assert!(cpu_table.get::<String>("model").is_err()); // Should not exist
        assert_eq!(cpu_table.get::<u32>("cores")?, 2);

        // Check memory section
        let memory_table = table.get::<Table>("memory")?;
        assert_eq!(memory_table.get::<u64>("total_mb")?, 4096);
        assert!(memory_table.get::<u64>("free_mb").is_err()); // Should not exist

        Ok(())
    }

    #[test]
    fn test_create_info_table_empty_data() -> mlua::Result<()> {
        let lua = create_lua()?;
        let info = HostInfo::default(); // All fields None

        let table = create_info_table(&lua, info)?;

        // All sections should exist but be empty
        let os_table = table.get::<Table>("os")?;
        assert!(os_table.get::<String>("distribution").is_err());
        assert!(os_table.get::<String>("version").is_err());
        assert!(os_table.get::<String>("family").is_err());
        assert!(os_table.get::<String>("kernel").is_err());
        assert!(os_table.get::<String>("hostname").is_err());

        let cpu_table = table.get::<Table>("cpu")?;
        assert!(cpu_table.get::<String>("model").is_err());
        assert!(cpu_table.get::<u32>("cores").is_err());

        let memory_table = table.get::<Table>("memory")?;
        assert!(memory_table.get::<u64>("total_mb").is_err());
        assert!(memory_table.get::<u64>("free_mb").is_err());

        Ok(())
    }

    // Tests for error scenarios and edge cases

    #[test]
    fn test_host_info_nil_input() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = host_info(&lua, Value::Nil);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e.to_string().contains("Host validation failed"));
        }

        Ok(())
    }

    #[test]
    fn test_host_info_invalid_table() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        // Missing required address field

        let result = host_info(&lua, Value::Table(host));
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(e.to_string().contains("Host validation failed"));
        }

        Ok(())
    }

    #[test]
    fn test_host_info_ssh_empty_private_key_file() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "example.com")?;
        host.set("user", "testuser")?;
        host.set("private_key_file", "")?; // Empty private key file

        // The function should return a table with "Unknown" values due to graceful error handling
        // rather than failing completely, since it implements single return value approach
        let result = host_info(&lua, Value::Table(host))?;

        // Verify the structure exists (graceful fallback)
        assert!(result.contains_key("os")?);
        assert!(result.contains_key("cpu")?);
        assert!(result.contains_key("memory")?);

        // Check that OS fields contain "Unknown" values due to connection failure
        let os_table = result.get::<Table>("os")?;
        if let Ok(distribution) = os_table.get::<String>("distribution") {
            assert_eq!(distribution, "Unknown");
        }

        Ok(())
    }

    // Tests for hostname fallback mechanisms

    #[test]
    fn test_parse_host_info_output_hostname_fallbacks() -> Result<()> {
        // Test that the parsing handles different hostname command outputs
        let sample_output_primary = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=primary-hostname
CPU_MODEL=Intel CPU
CPU_CORES=2
MEM_TOTAL_KB=2097152
MEM_AVAILABLE_KB=1048576"#;

        let info = parse_host_info_output(sample_output_primary);
        assert_eq!(info.os.hostname, Some("primary-hostname".to_string()));

        // Test with hostname that might come from fallback commands
        let sample_output_fallback = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=fallback-hostname.example.com
CPU_MODEL=Intel CPU
CPU_CORES=2
MEM_TOTAL_KB=2097152
MEM_AVAILABLE_KB=1048576"#;

        let info = parse_host_info_output(sample_output_fallback);
        assert_eq!(
            info.os.hostname,
            Some("fallback-hostname.example.com".to_string())
        );

        Ok(())
    }

    // Test memory conversion edge cases

    #[test]
    fn test_parse_host_info_output_memory_edge_cases() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=test-host
CPU_MODEL=Intel CPU
CPU_CORES=2
MEM_TOTAL_KB=1
MEM_AVAILABLE_KB=1023"#;

        let info = parse_host_info_output(sample_output);

        // Test very small memory values
        assert_eq!(info.memory.total_mb, Some(0)); // 1 / 1024 = 0 (integer division)
        assert_eq!(info.memory.free_mb, Some(0)); // 1023 / 1024 = 0 (integer division)

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_large_memory_values() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=test-host
CPU_MODEL=Intel CPU
CPU_CORES=64
MEM_TOTAL_KB=134217728
MEM_AVAILABLE_KB=67108864"#;

        let info = parse_host_info_output(sample_output);

        // Test large memory values (128GB total, 64GB available)
        assert_eq!(info.memory.total_mb, Some(131072)); // 134217728 / 1024
        assert_eq!(info.memory.free_mb, Some(65536)); // 67108864 / 1024

        Ok(())
    }

    // Test CPU core edge cases

    #[test]
    fn test_parse_host_info_output_cpu_edge_cases() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=test-host
CPU_MODEL=Single Core Processor
CPU_CORES=1
MEM_TOTAL_KB=1048576
MEM_AVAILABLE_KB=524288"#;

        let info = parse_host_info_output(sample_output);

        // Test single core system
        assert_eq!(info.cpu.cores, Some(1));
        assert_eq!(info.cpu.model, Some("Single Core Processor".to_string()));

        Ok(())
    }

    #[test]
    fn test_parse_host_info_output_many_cpu_cores() -> Result<()> {
        let sample_output = r#"DISTRO=Ubuntu
VERSION=22.04
FAMILY=ubuntu
KERNEL=5.15.0-101-generic
HOSTNAME=test-host
CPU_MODEL=High-End Server Processor
CPU_CORES=128
MEM_TOTAL_KB=1048576
MEM_AVAILABLE_KB=524288"#;

        let info = parse_host_info_output(sample_output);

        // Test many-core system
        assert_eq!(info.cpu.cores, Some(128));
        assert_eq!(
            info.cpu.model,
            Some("High-End Server Processor".to_string())
        );

        Ok(())
    }
}
