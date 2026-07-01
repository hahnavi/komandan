use mlua::Table;

/// Execution context for check functions
#[derive(Debug, Clone)]
pub enum ExecutionContext {
    /// Execute commands locally
    Local,
    /// Execute commands via SSH using the provided host configuration
    Remote(Table),
}

impl ExecutionContext {
    /// Create execution context from optional host table
    pub fn from_host_table(host_table: Option<&Table>) -> Self {
        host_table.map_or_else(|| Self::Local, |host| Self::Remote(host.clone()))
    }
}
