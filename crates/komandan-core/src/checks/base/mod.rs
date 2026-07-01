mod context;
mod result;

pub mod execution;
pub mod result_validation;
pub mod validation;

#[cfg(test)]
mod tests;

pub use context::ExecutionContext;
pub use result::CheckResult;

/// Escape single quotes for safe inclusion in a single-quoted shell argument.
pub fn shell_escape(input: &str) -> String {
    input.replace('\'', "'\"'\"'")
}
