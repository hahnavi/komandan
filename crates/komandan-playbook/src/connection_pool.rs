//! Per-host pooled connections (spec §6.2 step 4): one `ConnectionHandle`
//! per host, reused across the play's tasks rather than reconnecting each
//! task.

use std::collections::HashMap;

use komandan_plugin_abi::prelude::*;
use komandan_plugin_abi::{ConnectionHandle, CoreApiRef};
use thiserror::Error;

use crate::executors::Connection;

/// A connection pool keyed by host label. Borrows the host [`CoreApiRef`] for
/// the pool's lifetime.
#[derive(Debug)]
pub struct ConnectionPool<'core> {
    core: &'core CoreApiRef,
    handles: HashMap<String, ConnectionHandle>,
}

/// Failure to open a connection for a host.
#[derive(Debug, Error)]
#[error("failed to connect to {host}: {reason}")]
pub struct PoolError {
    /// Host label.
    pub host: String,
    /// Underlying reason (the host `CoreError` message).
    pub reason: String,
}

impl<'core> ConnectionPool<'core> {
    /// Build an empty pool borrowing `core`.
    #[must_use]
    pub fn new(core: &'core CoreApiRef) -> Self {
        Self {
            core,
            handles: HashMap::new(),
        }
    }

    /// The host `CoreApi` handle (for `report_record` etc.).
    #[must_use]
    pub const fn core(&self) -> &'core CoreApiRef {
        self.core
    }

    /// Acquire (creating on first use) a [`Connection`] for `host_label`.
    ///
    /// # Errors
    ///
    /// [`PoolError`] if `create_connection` fails on first contact.
    pub fn acquire(
        &mut self,
        host_label: &str,
        host: HostInfo,
    ) -> Result<Connection<'core>, PoolError> {
        let handle = if let Some(&h) = self.handles.get(host_label) {
            h
        } else {
            let h = self
                .core
                .create_connection(host.clone())
                .into_result()
                .map_err(|e| PoolError {
                    host: host_label.to_string(),
                    reason: e.message.to_string(),
                })?;
            self.handles.insert(host_label.to_string(), h);
            h
        };
        Ok(Connection::new(self.core, handle, host))
    }

    /// Close every cached connection (idempotent).
    pub fn close_all(&mut self) {
        for (_, h) in self.handles.drain() {
            self.core.close_connection(h);
        }
    }
}

impl Drop for ConnectionPool<'_> {
    fn drop(&mut self) {
        self.close_all();
    }
}
