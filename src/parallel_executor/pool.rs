use crate::connection::{Connection, create_connection};
use crate::parallel_executor::ConnectionStats;
use anyhow::Result;
use mlua::{Lua, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Connection pool for reusing connections across operations
#[derive(Debug)]
pub struct ConnectionPool {
    /// Pool of active connections keyed by host identifier
    connections: Arc<Mutex<HashMap<String, Arc<Mutex<Connection>>>>>,
    /// Connection usage statistics
    stats: Arc<Mutex<ConnectionStats>>,
    /// Maximum number of connections to pool
    max_connections: usize,
}

impl ConnectionPool {
    /// Creates a new connection pool
    #[must_use]
    pub fn new(max_connections: usize) -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(ConnectionStats {
                connections_created: 0,
                connections_reused: 0,
                reuse_ratio: 0.0,
                avg_connection_setup_time: Duration::ZERO,
            })),
            max_connections,
        }
    }

    /// Gets or creates a connection for the given host
    ///
    /// # Errors
    /// Returns an error if connection creation fails or host configuration is invalid
    pub fn get_connection(&self, lua: &Lua, host_value: &Value) -> Result<Arc<Mutex<Connection>>> {
        let host_key = Self::create_host_key(host_value);

        let mut connections = self
            .connections
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire connections lock"))?;
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire stats lock"))?;

        if let Some(connection) = connections.get(&host_key) {
            // Reuse existing connection
            stats.connections_reused += 1;
            #[allow(clippy::cast_precision_loss)]
            {
                stats.reuse_ratio = stats.connections_reused as f64
                    / (stats.connections_created + stats.connections_reused) as f64;
            }
            let connection_arc = Arc::clone(connection);
            drop(connections);
            drop(stats);
            return Ok(connection_arc);
        }

        // Create new connection if pool not full
        if connections.len() < self.max_connections {
            let start_time = Instant::now();
            let connection = create_connection(lua, host_value)?;
            let setup_time = start_time.elapsed();

            let connection_arc = Arc::new(Mutex::new(connection));
            connections.insert(host_key, Arc::clone(&connection_arc));

            stats.connections_created += 1;
            #[allow(clippy::cast_possible_truncation)]
            {
                stats.avg_connection_setup_time = Duration::from_nanos(
                    u64::try_from(
                        (((stats.avg_connection_setup_time.as_nanos()
                            * (stats.connections_created - 1) as u128)
                            + setup_time.as_nanos())
                            / stats.connections_created as u128)
                            .min(u128::from(u64::MAX)),
                    )
                    .unwrap_or(u64::MAX),
                );
            }
            #[allow(clippy::cast_precision_loss)]
            {
                stats.reuse_ratio = stats.connections_reused as f64
                    / (stats.connections_created + stats.connections_reused) as f64;
            }
            drop(connections);
            drop(stats);

            Ok(connection_arc)
        } else {
            // Pool is full, create temporary connection
            let connection = create_connection(lua, host_value)?;
            stats.connections_created += 1;
            drop(connections);
            drop(stats);
            Ok(Arc::new(Mutex::new(connection)))
        }
    }

    /// Creates a unique key for the host configuration
    fn create_host_key(host_value: &Value) -> String {
        match host_value {
            Value::Table(table) => {
                let address = table
                    .get::<String>("address")
                    .unwrap_or_else(|_| "localhost".to_string());
                let port = table.get::<u16>("port").unwrap_or(22);
                let user = table
                    .get::<String>("user")
                    .unwrap_or_else(|_| "default".to_string());
                let connection_type = table
                    .get::<String>("connection")
                    .unwrap_or_else(|_| "auto".to_string());

                format!("{connection_type}:{user}@{address}:{port}")
            }
            _ => "default".to_string(),
        }
    }

    /// Gets current connection statistics
    #[must_use]
    pub fn get_stats(&self) -> ConnectionStats {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }

    /// Clears all pooled connections
    pub fn clear(&self) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.clear();
        }
    }
}
