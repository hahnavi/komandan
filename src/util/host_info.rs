use crate::connection::create_connection;
use crate::validator::validate_host;
use mlua::{Lua, Table, Value, chunk};

#[derive(Debug, Default)]
pub struct HostInfo {
    pub os: OSInfo,
    pub cpu: CPUInfo,
    pub memory: MemoryInfo,
}

#[derive(Debug, Default)]
pub struct OSInfo {
    pub name: Option<String>,
    pub pretty_name: Option<String>,
    pub version: Option<String>,
    pub version_id: Option<String>,
    pub version_codename: Option<String>,
    pub id: Option<String>,
    pub id_like: Option<Vec<String>>,
    pub kernel: Option<String>,
    pub hostname: Option<String>,
}

#[derive(Debug, Default)]
pub struct CPUInfo {
    pub model: Option<String>,
    pub cores: Option<u32>,
}

#[derive(Debug, Default)]
pub struct MemoryInfo {
    pub total_mb: Option<u64>,
    pub free_mb: Option<u64>,
}

/// Gathers system information from a remote host using the centralized connection factory
///
/// This function uses the centralized connection factory to establish either SSH or local
/// connections based on host configuration, ensuring consistent authentication and error handling.
///
/// # Arguments
/// * `lua` - The Lua context for validation and table creation
/// * `host` - Host configuration (will be validated using existing validation logic)
///
/// # Returns
/// * `mlua::Result<Table>` - A table containing system information or "Unknown" values on failure
///
/// # Behavior
/// - Uses centralized connection factory for consistent SSH/local connection handling
/// - Gracefully handles connection failures by returning "Unknown" values
/// - Maintains backward compatibility with existing host parameter formats
/// - Provides detailed error logging while preserving function stability
pub fn host_info(lua: &Lua, host: Value) -> mlua::Result<Table> {
    // Handle nil host by defaulting to localhost, same as komando function
    let host_table = if host.is_nil() {
        lua.load(chunk! {
            return { address = "localhost" }
        })
        .eval::<Table>()?
    } else {
        validate_host(lua, host)
            .map_err(|e| mlua::Error::RuntimeError(format!("Host validation failed: {e}")))?
    };

    // Create connection using centralized factory with detailed error handling
    let mut connection = match create_connection(lua, &Value::Table(host_table)) {
        Ok(conn) => conn,
        Err(e) => {
            // Enhanced graceful error handling - log the specific error but return "Unknown" values
            // This maintains backward compatibility while providing better error context in logs
            eprintln!("Warning: Failed to connect to host for info gathering: {e}");
            return create_info_table(lua, create_unknown_host_info());
        }
    };

    // Execute information gathering script
    let script = r#"
{
  # OS Release information
  if [ -f /etc/os-release ]; then
    echo "OS_NAME=$(grep '^NAME=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_PRETTY_NAME=$(grep '^PRETTY_NAME=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_VERSION=$(grep '^VERSION=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_VERSION_ID=$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_VERSION_CODENAME=$(grep '^VERSION_CODENAME=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_ID=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
    echo "OS_ID_LIKE=$(grep '^ID_LIKE=' /etc/os-release | cut -d'=' -f2 | tr -d '\"' 2>/dev/null || echo 'Unknown')"
  else
    echo "OS_NAME=Unknown"
    echo "OS_PRETTY_NAME=Unknown"
    echo "OS_VERSION=Unknown"
    echo "OS_VERSION_ID=Unknown"
    echo "OS_VERSION_CODENAME=Unknown"
    echo "OS_ID=Unknown"
    echo "OS_ID_LIKE=Unknown"
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

    // Execute with graceful error handling
    let Ok((stdout, _stderr, exit_code)) = connection.cmd(script) else {
        return create_info_table(lua, create_unknown_host_info());
    };

    // Handle script execution failures gracefully
    if exit_code != 0 {
        return create_info_table(lua, create_unknown_host_info());
    }

    // Parse script output into structured data
    let info = parse_host_info_output(&stdout);

    // Convert to Lua table
    create_info_table(lua, info)
}

/// Creates a `HostInfo` structure with all fields set to "Unknown" values
/// Used when connection or script execution fails
pub fn create_unknown_host_info() -> HostInfo {
    HostInfo {
        os: OSInfo {
            name: Some("Unknown".to_string()),
            pretty_name: Some("Unknown".to_string()),
            version: Some("Unknown".to_string()),
            version_id: Some("Unknown".to_string()),
            version_codename: Some("Unknown".to_string()),
            id: Some("Unknown".to_string()),
            id_like: Some(vec!["Unknown".to_string()]),
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

pub fn parse_host_info_output(output: &str) -> HostInfo {
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
                "OS_NAME" => os_info.name = value,
                "OS_PRETTY_NAME" => os_info.pretty_name = value,
                "OS_VERSION" => os_info.version = value,
                "OS_VERSION_ID" => os_info.version_id = value,
                "OS_VERSION_CODENAME" => os_info.version_codename = value,
                "OS_ID" => os_info.id = value,
                "OS_ID_LIKE" => {
                    if let Some(id_like_str) = value {
                        // Split space-separated values into a vector
                        let id_like_vec: Vec<String> = id_like_str
                            .split_whitespace()
                            .map(std::string::ToString::to_string)
                            .collect();
                        os_info.id_like = if id_like_vec.is_empty() {
                            None
                        } else {
                            Some(id_like_vec)
                        };
                    }
                }
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

pub fn create_info_table(lua: &Lua, info: HostInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;

    // Create OS section
    let os_table = lua.create_table()?;
    os_table.set(
        "name",
        info.os.name.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set(
        "pretty_name",
        info.os.pretty_name.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set(
        "version",
        info.os.version.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set(
        "version_id",
        info.os.version_id.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set(
        "version_codename",
        info.os
            .version_codename
            .unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set("id", info.os.id.unwrap_or_else(|| "Unknown".to_string()))?;

    // Handle id_like as a table, defaulting to ["Unknown"] if None
    let id_like_table = if let Some(id_like) = info.os.id_like {
        lua.create_sequence_from(id_like)?
    } else {
        lua.create_sequence_from(vec!["Unknown".to_string()])?
    };
    os_table.set("id_like", id_like_table)?;

    os_table.set(
        "kernel",
        info.os.kernel.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    os_table.set(
        "hostname",
        info.os.hostname.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    table.set("os", os_table)?;

    // Create CPU section
    let cpu_table = lua.create_table()?;
    cpu_table.set(
        "model",
        info.cpu.model.unwrap_or_else(|| "Unknown".to_string()),
    )?;
    cpu_table.set("cores", info.cpu.cores.unwrap_or(0))?;
    table.set("cpu", cpu_table)?;

    // Create memory section
    let memory_table = lua.create_table()?;
    memory_table.set("total_mb", info.memory.total_mb.unwrap_or(0))?;
    memory_table.set("free_mb", info.memory.free_mb.unwrap_or(0))?;
    table.set("memory", memory_table)?;

    Ok(table)
}
