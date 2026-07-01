use mlua::Error::RuntimeError;

/// Errors raised while establishing an SSH or local connection.
///
/// The prior `troubleshooting` string fields, the `format_error` method, and
/// the `get_*_troubleshooting` helpers were verbose boilerplate that cluttered
/// every error message; they have been removed (see `REFACTOR_PLAN.md` §2.2).
/// `Display` now renders only the structured fields. Detailed SSH debugging
/// guidance belongs in documentation, not every error string.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectionError {
    /// Host table failed validation (missing/invalid fields).
    #[error("Host validation failed: {message} for host '{host}'")]
    HostValidation { message: String, host: String },

    /// SSH authentication failed.
    #[error("SSH authentication failed: {message} for {user}@{host}")]
    Authentication {
        message: String,
        host: String,
        user: String,
    },

    /// SSH TCP/session connection failed.
    #[error("SSH connection failed: {message} for {host}:{port}")]
    Connection {
        message: String,
        host: String,
        port: u16,
    },

    /// SSH host-key verification failed.
    #[error("SSH host key verification failed: {message} for host '{host}'")]
    HostKeyVerification { message: String, host: String },

    /// Connection-factory configuration error.
    #[error("SSH configuration error: {message} in {context}")]
    Configuration { message: String, context: String },
}

impl ConnectionError {
    /// Convert to an `mlua::Error::RuntimeError` carrying the `Display` output.
    ///
    /// Kept as a named helper because ~30 call sites use the
    /// `ConnectionError::Variant { ... }.to_runtime_error()` idiom at the mlua
    /// boundary; renaming/chaining them all to `.into()` would be pure churn.
    #[must_use]
    pub fn to_runtime_error(self) -> mlua::Error {
        RuntimeError(self.to_string())
    }
}
