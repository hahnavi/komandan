/// Error raised when a parallel-executor configuration parameter fails
/// validation.
///
/// Only the `Configuration` case is constructable today; the prior
/// `Serialization` / `Execution` / `Resource` / `LuaContext` /
/// `InputValidation` variants, the `to_lua_error` / `with_troubleshooting`
/// methods, and the manual `From` impl were unreachable scaffolding and have
/// been removed (see `REFACTOR_PLAN.md` §2.1). The remediation hints that used
/// to live on the variants were never surfaced (their only formatter,
/// `to_lua_error`, was uncalled); reintroduce a hint field only when a real
/// consumer (e.g. a `--explain` flag) needs it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ParallelExecutorError {
    /// A configuration parameter (thread count, chunk size, timeout, ...)
    /// failed validation in `Executor::validate_config`.
    #[error("{message}")]
    Configuration {
        /// What went wrong.
        message: String,
        /// Name of the offending parameter, when applicable.
        parameter: Option<String>,
    },
}
