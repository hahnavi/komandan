use crate::parallel_executor::{
    ExecutorConfig, ParallelExecutor, global_executor, validate_config,
};
use anyhow::{Context, Result};
use mlua::{Function, Lua, Table, Value};
use std::collections::HashMap;
use std::time::Duration;

/// Serialized representation of a Lua function for cross-thread execution
#[derive(Debug, Clone)]
pub struct SerializedFunction {
    /// The function bytecode
    pub bytecode: Vec<u8>,
    /// Serialized upvalues (captured variables)
    pub upvalues: Vec<SerializedValue>,
}

/// Serialized representation of Lua values
#[derive(Debug, Clone)]
pub enum SerializedValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Table(HashMap<String, Self>),
}

impl SerializedValue {
    /// Converts a Lua value to a serialized value
    ///
    /// # Errors
    /// Returns an error if the value type is not supported for serialization
    pub fn from_lua_value(value: Value) -> mlua::Result<Self> {
        match value {
            Value::Nil => Ok(Self::Nil),
            Value::Boolean(b) => Ok(Self::Boolean(b)),
            Value::Integer(i) => Ok(Self::Integer(i)),
            Value::Number(n) => Ok(Self::Number(n)),
            Value::String(s) => Ok(Self::String(s.to_str()?.to_string())),
            Value::Table(t) => {
                let mut map = HashMap::new();
                // Iterate with `Value` keys and stringify them so array-style
                // and integer-keyed tables (which are common in Lua) survive
                // the round-trip instead of erroring on a non-String key.
                for pair in t.pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    let key = key.to_string()?;
                    map.insert(key, Self::from_lua_value(value)?);
                }
                Ok(Self::Table(map))
            }
            _ => Err(mlua::Error::RuntimeError(
                "Unsupported value type for serialization".to_string(),
            )),
        }
    }

    /// Converts a serialized value back to a Lua value
    ///
    /// # Errors
    /// Returns an error if Lua value creation fails
    pub fn to_lua_value(&self, lua: &Lua) -> mlua::Result<Value> {
        match self {
            Self::Nil => Ok(Value::Nil),
            Self::Boolean(b) => Ok(Value::Boolean(*b)),
            Self::Integer(i) => Ok(Value::Integer(*i)),
            Self::Number(n) => Ok(Value::Number(*n)),
            Self::String(s) => Ok(Value::String(lua.create_string(s)?)),
            Self::Table(map) => {
                let table = lua.create_table()?;
                for (key, value) in map {
                    table.set(key.clone(), value.to_lua_value(lua)?)?;
                }
                Ok(Value::Table(table))
            }
        }
    }
}

/// Factory for creating isolated Lua contexts per thread
pub struct LuaContextFactory;

impl LuaContextFactory {
    /// Creates a new isolated Lua context for thread-safe execution
    ///
    /// # Returns
    /// * `Result<Lua>` - A new Lua context or an error
    ///
    /// # Errors
    /// Returns an error if Lua context creation fails
    pub fn create_isolated_context() -> Result<Lua> {
        let lua = Lua::new();

        // Set up the Komandan environment in the isolated context
        Self::setup_komandan_environment(&lua)
            .context("Failed to setup Komandan environment in isolated context")?;

        Ok(lua)
    }

    /// Sets up the Komandan environment in a Lua context
    ///
    /// # Arguments
    /// * `lua` - The Lua context to configure
    ///
    /// # Returns
    /// * `mlua::Result<()>` - Success or error
    fn setup_komandan_environment(lua: &Lua) -> mlua::Result<()> {
        // Import necessary functions from the main crate
        use crate::checks::collect_check_functions;
        use crate::defaults::Defaults;
        use crate::komando::{komando, komando_parallel_hosts, komando_parallel_tasks};
        use crate::modules::{base_module, collect_core_modules};
        use crate::util::{
            dprint, filter_hosts, host_info, parse_hosts_json_file, parse_hosts_json_url,
            regex_is_match,
        };

        // Create the komandan global table
        let komandan = lua.create_table()?;

        // Add defaults (create a new instance for this context)
        let defaults = Defaults::global();
        komandan.set("defaults", defaults)?;

        // Add base module
        let base_module = base_module(lua)?;
        komandan.set("KomandanModule", base_module)?;

        // Add core komando functions
        komandan.set("komando", lua.create_function(komando)?)?;
        komandan.set(
            "komando_parallel_tasks",
            lua.create_function(komando_parallel_tasks)?,
        )?;
        komandan.set(
            "komando_parallel_hosts",
            lua.create_function(komando_parallel_hosts)?,
        )?;

        // Add utility functions
        komandan.set("regex_is_match", lua.create_function(regex_is_match)?)?;
        komandan.set("filter_hosts", lua.create_function(filter_hosts)?)?;
        komandan.set(
            "parse_hosts_json_file",
            lua.create_function(parse_hosts_json_file)?,
        )?;
        komandan.set(
            "parse_hosts_json_url",
            lua.create_function(parse_hosts_json_url)?,
        )?;
        komandan.set("dprint", lua.create_function(dprint)?)?;
        komandan.set("host_info", lua.create_function(host_info)?)?;

        // Add core modules
        komandan.set("modules", collect_core_modules(lua)?)?;

        // Add check functions
        komandan.set("check", collect_check_functions(lua)?)?;

        // Set the global komandan table
        lua.globals().set("komandan", komandan.clone())?;

        // Create the 'k' alias table (not just a reference)
        let k_table = lua.create_table()?;

        // Copy core functionality to k
        k_table.set("defaults", komandan.get::<mlua::Value>("defaults")?)?;
        k_table.set("komando", komandan.get::<mlua::Value>("komando")?)?;
        k_table.set(
            "komando_parallel_hosts",
            komandan.get::<mlua::Value>("komando_parallel_hosts")?,
        )?;
        k_table.set(
            "komando_parallel_tasks",
            komandan.get::<mlua::Value>("komando_parallel_tasks")?,
        )?;

        // Copy utility functions
        k_table.set(
            "regex_is_match",
            komandan.get::<mlua::Value>("regex_is_match")?,
        )?;
        k_table.set("filter_hosts", komandan.get::<mlua::Value>("filter_hosts")?)?;
        k_table.set(
            "parse_hosts_json_file",
            komandan.get::<mlua::Value>("parse_hosts_json_file")?,
        )?;
        k_table.set(
            "parse_hosts_json_url",
            komandan.get::<mlua::Value>("parse_hosts_json_url")?,
        )?;
        k_table.set("dprint", komandan.get::<mlua::Value>("dprint")?)?;
        k_table.set("host_info", komandan.get::<mlua::Value>("host_info")?)?;

        // Create alias 'k.mods' for 'komandan.modules'
        let modules_table = komandan.get::<mlua::Table>("modules")?;
        k_table.set("mods", modules_table)?;

        // Create alias 'k.check' for 'komandan.check'
        let check_table = komandan.get::<mlua::Table>("check")?;
        k_table.set("check", check_table)?;

        // Set the k global
        lua.globals().set("k", k_table)?;

        Ok(())
    }

    /// Serializes a Lua function for cross-thread execution.
    ///
    /// **Upvalues are not captured.** `mlua`'s `dump` only emits bytecode, so
    /// any captured locals would be silently lost on the receiving side, which
    /// is a footgun. To make that contract explicit we reject any function
    /// whose upvalue count is non-zero — callers must pass a pure function
    /// (or wrap their captured state into the data table).
    ///
    /// # Arguments
    /// * `lua` - The source Lua context
    /// * `func` - The function to serialize
    ///
    /// # Returns
    /// * `mlua::Result<SerializedFunction>` - Serialized function or error
    ///
    /// # Errors
    /// Returns an error if:
    /// - The function has one or more upvalues (captured locals)
    /// - Function introspection fails
    pub fn serialize_function(_lua: &Lua, func: &Function) -> mlua::Result<SerializedFunction> {
        // Dump the function to bytecode (no upvalue capture happens here).
        let bytecode = func.dump(false);

        // Reject functions with upvalues so callers get a clear failure
        // instead of silently-dropped captured state on the deserialize side.
        let info = func.info();
        if info.num_upvalues > 0 {
            return Err(mlua::Error::RuntimeError(format!(
                "Functions with upvalues cannot be serialized for parallel execution \
                 (found {} upvalue(s)). Refactor to pass captured state via the data table.",
                info.num_upvalues
            )));
        }

        Ok(SerializedFunction {
            bytecode,
            upvalues: Vec::new(),
        })
    }

    /// Deserializes a function in an isolated Lua context.
    ///
    /// The bytecode produced by [`serialize_function`] carries no upvalues, so
    /// nothing needs to be restored here.
    ///
    /// # Arguments
    /// * `lua` - The target Lua context
    /// * `serialized_func` - The serialized function
    ///
    /// # Returns
    /// * `mlua::Result<Function>` - Deserialized function or error
    ///
    /// # Errors
    /// Returns an error if function deserialization fails
    pub fn deserialize_function(
        lua: &Lua,
        serialized_func: &SerializedFunction,
    ) -> mlua::Result<Function> {
        // Load the function from bytecode (no upvalue restore: see serialize_function).
        let func = lua.load(&serialized_func.bytecode).into_function()?;

        Ok(func)
    }
}

/// Result of executing a function in parallel
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Index of the data element this result corresponds to
    pub index: usize,
    /// The result of the function execution (serialized)
    pub result: Result<SerializedValue, String>,
    /// Time taken to execute the function
    pub execution_time: Duration,
    /// Thread ID that executed this function
    pub thread_id: Option<String>, // Use String instead of ThreadId for Send
}

impl ExecutionResult {
    /// Creates a successful execution result
    #[must_use]
    pub fn success(index: usize, result: SerializedValue, execution_time: Duration) -> Self {
        Self {
            index,
            result: Ok(result),
            execution_time,
            thread_id: Some(format!("{:?}", std::thread::current().id())),
        }
    }

    /// Creates a failed execution result
    #[must_use]
    pub fn failure(index: usize, error: String, execution_time: Duration) -> Self {
        Self {
            index,
            result: Err(error),
            execution_time,
            thread_id: Some(format!("{:?}", std::thread::current().id())),
        }
    }

    /// Converts the execution result to a Lua table
    ///
    /// # Errors
    /// Returns an error if Lua table creation fails
    pub fn to_lua_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let table = lua.create_table()?;

        match &self.result {
            Ok(serialized_value) => {
                table.set("success", true)?;
                let lua_value = serialized_value.to_lua_value(lua)?;
                table.set("result", lua_value)?;
            }
            Err(error) => {
                table.set("success", false)?;
                table.set("error", error.clone())?;
            }
        }

        table.set("execution_time", self.execution_time.as_secs_f64())?;

        if let Some(thread_id) = &self.thread_id {
            table.set("thread_id", thread_id.clone())?;
        }

        Ok(table)
    }
}
/// Creates a new parallel executor instance (constructor approach)
///
/// # Arguments
/// * `lua` - The Lua context
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - Function that creates executor instances
///
/// # Errors
/// Returns an error if function creation fails
pub fn parallel_executor_constructor(lua: &Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, config_table: Option<Table>| {
        let config = if let Some(table) = config_table {
            let thread_count = table.get::<Option<usize>>("thread_count")?;
            let chunk_size = table.get::<Option<usize>>("chunk_size")?;
            let timeout_seconds = table.get::<Option<u64>>("timeout_seconds")?;
            let error_strategy = table.get::<Option<String>>("error_strategy")?;
            let max_memory_mb = table.get::<Option<usize>>("max_memory_mb")?;

            Some(ExecutorConfig {
                thread_count,
                chunk_size,
                timeout_seconds,
                error_strategy,
                max_memory_mb,
            })
        } else {
            None
        };

        let executor =
            ParallelExecutor::new(config).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        lua.create_userdata(executor)
    })
}
/// Creates the global parallel executor interface for Lua
///
/// # Arguments
/// * `lua` - The Lua context
///
/// # Returns
/// * `mlua::Result<Table>` - Table with map and configure methods
///
/// # Errors
/// Returns an error if interface creation fails
pub fn create_global_executor_interface(lua: &Lua) -> mlua::Result<Table> {
    let interface = lua.create_table()?;

    // Add map method
    let map_fn = lua.create_function(|lua, (_self, data, func): (Table, Table, Function)| {
        let executor = global_executor()?;
        let executor = executor.lock().map_err(|e| {
            mlua::Error::RuntimeError(format!("Failed to lock global executor: {e}"))
        })?;

        executor.map(lua, &data, &func)
    })?;
    interface.set("map", map_fn)?;

    // Add configure method
    let configure_fn = lua.create_function(|_lua, (_self, config_table): (Table, Table)| {
        let thread_count = config_table.get::<Option<usize>>("thread_count")?;
        let chunk_size = config_table.get::<Option<usize>>("chunk_size")?;
        let timeout_seconds = config_table.get::<Option<u64>>("timeout_seconds")?;
        let error_strategy = config_table.get::<Option<String>>("error_strategy")?;
        let max_memory_mb = config_table.get::<Option<usize>>("max_memory_mb")?;

        let config = ExecutorConfig {
            thread_count,
            chunk_size,
            timeout_seconds,
            error_strategy,
            max_memory_mb,
        };

        let executor = global_executor()?;
        let mut executor = executor.lock().map_err(|e| {
            mlua::Error::RuntimeError(format!("Failed to lock global executor: {e}"))
        })?;

        executor.configure(config)
    })?;
    interface.set("configure", configure_fn)?;

    Ok(interface)
}

/// Implements userdata methods for `ParallelExecutor`
impl mlua::UserData for ParallelExecutor {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("map", |lua, this, (data, func): (Table, Function)| {
            this.map(lua, &data, &func)
        });

        methods.add_method_mut("configure", |_lua, this, config_table: Table| {
            let thread_count = config_table.get::<Option<usize>>("thread_count")?;
            let chunk_size = config_table.get::<Option<usize>>("chunk_size")?;
            let timeout_seconds = config_table.get::<Option<u64>>("timeout_seconds")?;
            let error_strategy = config_table.get::<Option<String>>("error_strategy")?;
            let max_memory_mb = config_table.get::<Option<usize>>("max_memory_mb")?;

            let config = ExecutorConfig {
                thread_count,
                chunk_size,
                timeout_seconds,
                error_strategy,
                max_memory_mb,
            };

            this.configure(config)
        });

        methods.add_method("thread_count", |_lua, this, ()| Ok(this.thread_count()));

        methods.add_method("config", |lua, this, ()| {
            let config = this.config();
            let table = lua.create_table()?;

            if let Some(thread_count) = config.thread_count {
                table.set("thread_count", thread_count)?;
            }
            if let Some(chunk_size) = config.chunk_size {
                table.set("chunk_size", chunk_size)?;
            }
            if let Some(timeout_seconds) = config.timeout_seconds {
                table.set("timeout_seconds", timeout_seconds)?;
            }
            if let Some(error_strategy) = &config.error_strategy {
                table.set("error_strategy", error_strategy.clone())?;
            }
            if let Some(max_memory_mb) = config.max_memory_mb {
                table.set("max_memory_mb", max_memory_mb)?;
            }

            // Add effective values
            table.set("effective_thread_count", config.effective_thread_count())?;
            table.set("effective_chunk_size", config.effective_chunk_size())?;
            table.set(
                "effective_timeout_seconds",
                config.effective_timeout_seconds(),
            )?;
            table.set(
                "effective_error_strategy",
                config.effective_error_strategy(),
            )?;
            table.set("effective_max_memory_mb", config.effective_max_memory_mb())?;

            Ok(table)
        });

        methods.add_method("validate_config", |_lua, _this, config_table: Table| {
            let thread_count = config_table.get::<Option<usize>>("thread_count")?;
            let chunk_size = config_table.get::<Option<usize>>("chunk_size")?;
            let timeout_seconds = config_table.get::<Option<u64>>("timeout_seconds")?;
            let error_strategy = config_table.get::<Option<String>>("error_strategy")?;
            let max_memory_mb = config_table.get::<Option<usize>>("max_memory_mb")?;

            let config = ExecutorConfig {
                thread_count,
                chunk_size,
                timeout_seconds,
                error_strategy,
                max_memory_mb,
            };

            validate_config(&config).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

            Ok(true)
        });
    }
}
