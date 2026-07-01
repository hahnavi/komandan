//! Error types for the playbook plugin.

use thiserror::Error;

/// A playbook parse / structural error.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Malformed YAML (`serde_yaml` could not build the value tree).
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    /// A file could not be read or located.
    #[error("{0}")]
    Load(String),
    /// A play-level structural problem (missing `hosts:`, bad shape, ...).
    #[error("{0}")]
    Play(String),
    /// A task-level structural problem (no module key, multiple module keys,
    /// bad block, ...).
    #[error("task: {0}")]
    Task(String),
}

impl ParseError {
    /// Wrap a [`serde_yaml::Error`].
    #[must_use]
    pub const fn yaml(e: serde_yaml::Error) -> Self {
        Self::Yaml(e)
    }

    /// A file-load error (missing, unreadable, ...).
    #[must_use]
    pub fn load(msg: impl Into<String>) -> Self {
        Self::Load(msg.into())
    }

    /// A play-level error.
    #[must_use]
    pub fn play(msg: impl Into<String>) -> Self {
        Self::Play(msg.into())
    }

    /// A task-level error.
    #[must_use]
    pub fn task(msg: impl Into<String>) -> Self {
        Self::Task(msg.into())
    }
}
