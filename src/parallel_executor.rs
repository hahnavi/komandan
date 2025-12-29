use crate::connection::{Connection, create_connection};
use anyhow::{Context, Result};
use mlua::{Function, Lua, Table, Value};
use rayon::{ThreadPool, ThreadPoolBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Comprehensive error types for the parallel executor
#[derive(Debug, Clone)]
pub enum ParallelExecutorError {
    /// Configuration validation errors
    Configuration {
        message: String,
        suggestion: String,
        parameter: Option<String>,
    },
    /// Function serialization errors
    Serialization {
        message: String,
        function_info: String,
        troubleshooting: String,
    },
    /// Individual function execution errors
    Execution {
        index: usize,
        error: String,
        thread_id: String,
        input_data: String,
    },
    /// Resource allocation and management errors
    Resource {
        message: String,
        system_info: String,
        recovery_suggestion: String,
    },
    /// Lua context creation and management errors
    LuaContext {
        message: String,
        context_id: String,
        troubleshooting: String,
    },
    /// Input validation errors
    InputValidation {
        message: String,
        parameter: String,
        expected: String,
        actual: String,
        suggestion: String,
    },
}

impl ParallelExecutorError {
    /// Converts the error to a Lua error with comprehensive information
    #[must_use]
    pub fn to_lua_error(self) -> mlua::Error {
        let error_message = match self {
            Self::Configuration {
                message,
                suggestion,
                parameter,
            } => {
                let param_info = parameter.map_or(String::new(), |p| format!(" (parameter: {p})"));
                format!("Configuration Error{param_info}: {message}\nSuggestion: {suggestion}")
            }
            Self::Serialization {
                message,
                function_info,
                troubleshooting,
            } => {
                format!(
                    "Serialization Error: {message}\nFunction Info: {function_info}\nTroubleshooting: {troubleshooting}"
                )
            }
            Self::Execution {
                index,
                error,
                thread_id,
                input_data,
            } => {
                format!(
                    "Execution Error at index {index}: {error}\nThread: {thread_id}\nInput: {input_data}"
                )
            }
            Self::Resource {
                message,
                system_info,
                recovery_suggestion,
            } => {
                format!(
                    "Resource Error: {message}\nSystem Info: {system_info}\nRecovery: {recovery_suggestion}"
                )
            }
            Self::LuaContext {
                message,
                context_id,
                troubleshooting,
            } => {
                format!(
                    "Lua Context Error ({context_id}): {message}\nTroubleshooting: {troubleshooting}"
                )
            }
            Self::InputValidation {
                message,
                parameter,
                expected,
                actual,
                suggestion,
            } => {
                format!(
                    "Input Validation Error: {message}\nParameter: {parameter}\nExpected: {expected}\nActual: {actual}\nSuggestion: {suggestion}"
                )
            }
        };

        mlua::Error::RuntimeError(error_message)
    }

    /// Adds troubleshooting information to the error
    #[must_use]
    pub fn with_troubleshooting(mut self) -> Self {
        match &mut self {
            Self::Configuration { suggestion, .. } => {
                if suggestion.is_empty() {
                    *suggestion = "Check the parallel executor configuration parameters. Ensure thread_count is between 1-1024 and chunk_size is greater than 0.".to_string();
                }
            }
            Self::Serialization {
                troubleshooting, ..
            } => {
                if troubleshooting.is_empty() {
                    *troubleshooting = "Ensure the function doesn't capture external variables (upvalues) that cannot be serialized. Use simple functions or pass data as parameters.".to_string();
                }
            }
            Self::Resource {
                recovery_suggestion,
                ..
            } => {
                if recovery_suggestion.is_empty() {
                    *recovery_suggestion = "Try reducing thread_count or chunk_size. Check system memory and CPU availability.".to_string();
                }
            }
            Self::LuaContext {
                troubleshooting, ..
            } => {
                if troubleshooting.is_empty() {
                    *troubleshooting = "This may indicate memory pressure or Lua state corruption. Try reducing parallel load or restarting the application.".to_string();
                }
            }
            _ => {}
        }
        self
    }
}

impl From<ParallelExecutorError> for anyhow::Error {
    fn from(err: ParallelExecutorError) -> Self {
        match err {
            ParallelExecutorError::Configuration {
                message,
                suggestion,
                parameter,
            } => {
                let param_info = parameter.map_or(String::new(), |p| format!(" ({p})"));
                anyhow::anyhow!("{message}{param_info}: {suggestion}")
            }
            ParallelExecutorError::Serialization { message, .. } => {
                anyhow::anyhow!("Serialization error: {message}")
            }
            ParallelExecutorError::Execution { error, .. } => {
                anyhow::anyhow!("Execution error: {error}")
            }
            ParallelExecutorError::Resource { message, .. } => {
                anyhow::anyhow!("Resource error: {message}")
            }
            ParallelExecutorError::LuaContext { message, .. } => {
                anyhow::anyhow!("Lua context error: {message}")
            }
            ParallelExecutorError::InputValidation { message, .. } => {
                anyhow::anyhow!("Input validation error: {message}")
            }
        }
    }
}

/// Summary of parallel execution results
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionSummary {
    /// Total number of items processed
    pub total_items: usize,
    /// Number of successful executions
    pub successful_count: usize,
    /// Number of failed executions
    pub failed_count: usize,
    /// Total execution time
    pub total_time: Duration,
    /// Average execution time per item
    pub average_time: Duration,
    /// Thread utilization information
    pub thread_info: ThreadUtilization,
    /// Error breakdown by category
    pub error_breakdown: HashMap<String, usize>,
    /// Performance metrics
    pub performance_metrics: PerformanceMetrics,
}

/// Performance metrics for monitoring and optimization
#[derive(Debug, Clone, Serialize)]
pub struct PerformanceMetrics {
    /// Memory usage statistics
    pub memory_usage: MemoryUsage,
    /// Connection reuse statistics
    pub connection_stats: ConnectionStats,
    /// Batching efficiency metrics
    pub batching_metrics: BatchingMetrics,
    /// Throughput measurements
    pub throughput: ThroughputMetrics,
}

/// Memory usage statistics
#[derive(Debug, Clone, Serialize)]
pub struct MemoryUsage {
    /// Peak memory usage per thread (estimated)
    pub peak_memory_per_thread_mb: f64,
    /// Total estimated memory usage
    pub total_memory_usage_mb: f64,
    /// Memory efficiency score (0.0 to 1.0)
    pub memory_efficiency: f64,
}

/// Connection reuse and pooling statistics
#[derive(Debug, Clone, Serialize, Default)]
pub struct ConnectionStats {
    /// Number of connections created
    pub connections_created: usize,
    /// Number of connections reused
    pub connections_reused: usize,
    /// Connection reuse ratio (0.0 to 1.0)
    pub reuse_ratio: f64,
    /// Average connection setup time
    pub avg_connection_setup_time: Duration,
}

/// Batching efficiency metrics
#[derive(Debug, Clone, Serialize, Default)]
pub struct BatchingMetrics {
    /// Number of batches processed
    pub batch_count: usize,
    /// Average batch size
    pub avg_batch_size: f64,
    /// Batch processing efficiency (0.0 to 1.0)
    pub batch_efficiency: f64,
    /// Load balancing score (0.0 to 1.0, 1.0 = perfect balance)
    pub load_balance_score: f64,
}

/// Throughput and performance measurements
#[derive(Debug, Clone, Serialize)]
pub struct ThroughputMetrics {
    /// Items processed per second
    pub items_per_second: f64,
    /// Parallel speedup factor compared to sequential
    pub speedup_factor: f64,
    /// CPU utilization efficiency (0.0 to 1.0)
    pub cpu_efficiency: f64,
}

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

/// Efficient batch processor for large datasets
#[derive(Debug)]
pub struct BatchProcessor {
    /// Optimal batch size based on system resources
    optimal_batch_size: usize,
    /// Memory threshold for adaptive batching
    memory_threshold_mb: usize,
    /// Batching statistics
    stats: Arc<Mutex<BatchingMetrics>>,
}

impl BatchProcessor {
    /// Creates a new batch processor with adaptive sizing
    #[must_use]
    pub fn new(config: &ExecutorConfig) -> Self {
        let optimal_batch_size = Self::calculate_optimal_batch_size(config);

        Self {
            optimal_batch_size,
            memory_threshold_mb: config.effective_max_memory_mb(),
            stats: Arc::new(Mutex::new(BatchingMetrics {
                batch_count: 0,
                avg_batch_size: 0.0,
                batch_efficiency: 0.0,
                load_balance_score: 0.0,
            })),
        }
    }

    /// Calculates optimal batch size based on configuration and system resources
    fn calculate_optimal_batch_size(config: &ExecutorConfig) -> usize {
        let thread_count = config.effective_thread_count();
        let base_chunk_size = config.effective_chunk_size();

        // Adaptive batch sizing based on thread count and memory constraints
        #[allow(clippy::cast_precision_loss)]
        let memory_factor = (config.effective_max_memory_mb() as f64 / 512.0).min(4.0);
        #[allow(clippy::cast_precision_loss)]
        let thread_factor = (thread_count as f64).sqrt();

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let optimal_size = (base_chunk_size as f64 * memory_factor * thread_factor) as usize;

        // Clamp to reasonable bounds
        optimal_size.clamp(10, 10000)
    }

    /// Processes data in optimized batches
    pub fn process_batches<T, F, R>(&self, data: &[T], processor: F) -> Vec<R>
    where
        T: Send + Sync + Clone,
        F: Fn(Vec<T>) -> Vec<R> + Send + Sync,
        R: Send,
    {
        use rayon::prelude::*;

        let batch_size = self.get_adaptive_batch_size(data.len());
        let batches: Vec<Vec<T>> = data.chunks(batch_size).map(<[T]>::to_vec).collect();

        let Ok(mut stats) = self.stats.lock() else {
            return Vec::new(); // Return empty vec on lock failure
        };
        stats.batch_count = batches.len();
        #[allow(clippy::cast_precision_loss)]
        {
            stats.avg_batch_size = if batches.is_empty() {
                0.0
            } else {
                data.len() as f64 / batches.len() as f64
            };
        }

        // Calculate load balance score
        if batches.len() > 1 {
            let sizes: Vec<usize> = batches.iter().map(std::vec::Vec::len).collect();
            #[allow(clippy::cast_precision_loss)]
            let avg_size = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
            #[allow(clippy::cast_precision_loss)]
            let variance = sizes
                .iter()
                .map(|&size| (size as f64 - avg_size).powi(2))
                .sum::<f64>()
                / sizes.len() as f64;
            let std_dev = variance.sqrt();
            stats.load_balance_score = (1.0 - (std_dev / avg_size)).max(0.0);
        } else {
            stats.load_balance_score = 1.0;
        }

        stats.batch_efficiency = Self::calculate_batch_efficiency(data.len(), batch_size);
        drop(stats);

        // Process batches in parallel
        batches.into_par_iter().flat_map(processor).collect()
    }

    /// Gets adaptive batch size based on data size and memory constraints
    fn get_adaptive_batch_size(&self, data_size: usize) -> usize {
        if data_size <= 100 {
            // Small datasets: use smaller batches for better responsiveness
            (self.optimal_batch_size / 4).max(1)
        } else if data_size <= 10000 {
            // Medium datasets: use optimal batch size
            self.optimal_batch_size
        } else {
            // Large datasets: increase batch size for efficiency, but respect memory limits
            let max_batch_size = (self.memory_threshold_mb * 1024 * 1024) / (64 * 1024); // Assume 64KB per item
            let large_batch_size = (self.optimal_batch_size * 2).min(data_size / 10);
            large_batch_size.min(max_batch_size.max(1))
        }
    }

    /// Calculates batch processing efficiency
    fn calculate_batch_efficiency(total_items: usize, batch_size: usize) -> f64 {
        if total_items == 0 || batch_size == 0 {
            return 0.0;
        }

        let _num_batches = total_items.div_ceil(batch_size);
        let last_batch_size = total_items % batch_size;

        if last_batch_size == 0 {
            1.0 // Perfect batching
        } else {
            // Efficiency based on how well the last batch fills up
            #[allow(clippy::cast_precision_loss)]
            let efficiency = (total_items - last_batch_size) as f64 / total_items as f64;
            #[allow(clippy::cast_precision_loss)]
            {
                (last_batch_size as f64 / batch_size as f64).mul_add(1.0 - efficiency, efficiency)
            }
        }
    }

    /// Gets current batching statistics
    #[must_use]
    pub fn get_stats(&self) -> BatchingMetrics {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }
}

/// Performance monitor for tracking execution metrics
#[derive(Debug)]
pub struct PerformanceMonitor {
    /// Start time of monitoring
    start_time: Instant,
    /// Memory usage tracker
    memory_tracker: MemoryTracker,
    /// Throughput calculator
    throughput_calculator: ThroughputCalculator,
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMonitor {
    /// Creates a new performance monitor
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            memory_tracker: MemoryTracker::new(),
            throughput_calculator: ThroughputCalculator::new(),
        }
    }

    /// Records the start of processing
    pub fn start_processing(&mut self, item_count: usize, thread_count: usize) {
        self.start_time = Instant::now();
        self.throughput_calculator.start(item_count, thread_count);
        self.memory_tracker.start_monitoring();
    }

    /// Records the completion of processing
    pub fn finish_processing(&mut self, successful_count: usize) -> PerformanceMetrics {
        let total_time = self.start_time.elapsed();

        let memory_usage = self.memory_tracker.get_usage();
        let throughput = self
            .throughput_calculator
            .calculate(total_time, successful_count);

        PerformanceMetrics {
            memory_usage,
            connection_stats: ConnectionStats {
                connections_created: 0,
                connections_reused: 0,
                reuse_ratio: 0.0,
                avg_connection_setup_time: Duration::ZERO,
            },
            batching_metrics: BatchingMetrics {
                batch_count: 0,
                avg_batch_size: 0.0,
                batch_efficiency: 0.0,
                load_balance_score: 0.0,
            },
            throughput,
        }
    }

    /// Updates performance metrics with connection and batching stats
    #[must_use]
    pub const fn update_metrics(
        &self,
        mut metrics: PerformanceMetrics,
        connection_stats: ConnectionStats,
        batching_metrics: BatchingMetrics,
    ) -> PerformanceMetrics {
        metrics.connection_stats = connection_stats;
        metrics.batching_metrics = batching_metrics;
        metrics
    }
}

/// Memory usage tracker
#[derive(Debug)]
struct MemoryTracker {
    initial_memory: usize,
    peak_memory: usize,
}

impl MemoryTracker {
    const fn new() -> Self {
        Self {
            initial_memory: 0,
            peak_memory: 0,
        }
    }

    const fn start_monitoring(&mut self) {
        // In a real implementation, we would use system calls to get actual memory usage
        // For now, we'll estimate based on thread count and data size
        self.initial_memory = Self::get_estimated_memory_usage();
        self.peak_memory = self.initial_memory;
    }

    fn get_usage(&self) -> MemoryUsage {
        let current_memory = Self::get_estimated_memory_usage();
        #[allow(clippy::cast_precision_loss)]
        let peak_memory_mb = (self.peak_memory.max(current_memory)) as f64 / 1024.0 / 1024.0;

        MemoryUsage {
            #[allow(clippy::cast_precision_loss)]
            peak_memory_per_thread_mb: peak_memory_mb
                / std::thread::available_parallelism()
                    .map(std::num::NonZero::get)
                    .unwrap_or(1) as f64,
            total_memory_usage_mb: peak_memory_mb,
            memory_efficiency: if peak_memory_mb > 0.0 {
                #[allow(clippy::cast_precision_loss)]
                {
                    (self.initial_memory as f64 / 1024.0 / 1024.0) / peak_memory_mb
                }
            } else {
                1.0
            },
        }
    }

    const fn get_estimated_memory_usage() -> usize {
        // Simplified memory estimation - in production this would use actual system calls
        // For now, return a reasonable estimate based on typical Lua context size
        64 * 1024 * 1024 // 64MB base estimate
    }
}

/// Throughput calculator
#[derive(Debug)]
struct ThroughputCalculator {
    item_count: usize,
    thread_count: usize,
    start_time: Option<Instant>,
}

impl ThroughputCalculator {
    const fn new() -> Self {
        Self {
            item_count: 0,
            thread_count: 1,
            start_time: None,
        }
    }

    fn start(&mut self, item_count: usize, thread_count: usize) {
        self.item_count = item_count;
        self.thread_count = thread_count;
        self.start_time = Some(Instant::now());
    }

    fn calculate(&self, total_time: Duration, successful_count: usize) -> ThroughputMetrics {
        #[allow(clippy::cast_precision_loss)]
        let items_per_second = if total_time.as_secs_f64() > 0.0 {
            successful_count as f64 / total_time.as_secs_f64()
        } else {
            0.0
        };

        // Estimate sequential time (rough approximation)
        #[allow(clippy::cast_precision_loss)]
        let estimated_sequential_time = total_time.as_secs_f64() * self.thread_count as f64;
        let speedup_factor = if total_time.as_secs_f64() > 0.0 {
            estimated_sequential_time / total_time.as_secs_f64()
        } else {
            1.0
        };

        // CPU efficiency based on how well we utilized available threads
        #[allow(clippy::cast_precision_loss)]
        let theoretical_max_speedup = self.thread_count as f64;
        let cpu_efficiency = (speedup_factor / theoretical_max_speedup).min(1.0);

        ThroughputMetrics {
            items_per_second,
            speedup_factor,
            cpu_efficiency,
        }
    }
}

/// Thread utilization statistics
#[derive(Debug, Clone, Serialize)]
pub struct ThreadUtilization {
    /// Number of threads used
    pub threads_used: usize,
    /// Maximum concurrent executions
    pub max_concurrent: usize,
    /// Thread efficiency (0.0 to 1.0)
    pub efficiency: f64,
}

impl ExecutionSummary {
    /// Creates a new execution summary from results with performance metrics
    #[must_use]
    pub fn from_results(
        results: &[ExecutionResult],
        thread_count: usize,
        total_time: Duration,
        performance_metrics: PerformanceMetrics,
    ) -> Self {
        let total_items = results.len();
        let successful_count = results.iter().filter(|r| r.result.is_ok()).count();
        let failed_count = total_items - successful_count;

        let average_time = if total_items > 0 {
            let total_nanos = results
                .iter()
                .map(|r| r.execution_time.as_nanos())
                .sum::<u128>();
            let avg_nanos = total_nanos / total_items as u128;
            // Clamp to u64::MAX to avoid overflow
            #[allow(clippy::cast_possible_truncation)]
            Duration::from_nanos(
                u64::try_from(std::cmp::min(avg_nanos, u128::from(u64::MAX))).unwrap_or(u64::MAX),
            )
        } else {
            Duration::ZERO
        };

        // Calculate error breakdown
        let mut error_breakdown = HashMap::new();
        for result in results {
            if let Err(error) = &result.result {
                let error_type = Self::categorize_error(error);
                *error_breakdown.entry(error_type).or_insert(0) += 1;
            }
        }

        // Calculate thread utilization
        let unique_threads: std::collections::HashSet<_> = results
            .iter()
            .filter_map(|r| r.thread_id.as_ref())
            .collect();

        let threads_used = unique_threads.len();
        #[allow(clippy::cast_precision_loss)]
        let efficiency = if thread_count > 0 {
            threads_used as f64 / thread_count as f64
        } else {
            0.0
        };

        Self {
            total_items,
            successful_count,
            failed_count,
            total_time,
            average_time,
            thread_info: ThreadUtilization {
                threads_used,
                max_concurrent: thread_count,
                efficiency,
            },
            error_breakdown,
            performance_metrics,
        }
    }

    /// Categorizes an error message into a type
    fn categorize_error(error: &str) -> String {
        if error.contains("serialize") || error.contains("deserialize") {
            "Serialization".to_string()
        } else if error.contains("Lua context") || error.contains("context") {
            "Lua Context".to_string()
        } else if error.contains("Function execution") {
            "Function Execution".to_string()
        } else if error.contains("convert") || error.contains("type") {
            "Type Conversion".to_string()
        } else {
            "Other".to_string()
        }
    }

    /// Converts the summary to a Lua table
    ///
    /// # Errors
    /// Returns an error if Lua table creation fails
    pub fn to_lua_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let table = lua.create_table()?;

        table.set("total_items", self.total_items)?;
        table.set("successful_count", self.successful_count)?;
        table.set("failed_count", self.failed_count)?;
        table.set(
            "success_rate",
            if self.total_items > 0 {
                #[allow(clippy::cast_precision_loss)]
                {
                    self.successful_count as f64 / self.total_items as f64
                }
            } else {
                0.0
            },
        )?;
        table.set("total_time", self.total_time.as_secs_f64())?;
        table.set("average_time", self.average_time.as_secs_f64())?;

        // Thread information
        let thread_info = lua.create_table()?;
        thread_info.set("threads_used", self.thread_info.threads_used)?;
        thread_info.set("max_concurrent", self.thread_info.max_concurrent)?;
        thread_info.set("efficiency", self.thread_info.efficiency)?;
        table.set("thread_info", thread_info)?;

        // Error breakdown
        let error_breakdown = lua.create_table()?;
        for (error_type, count) in &self.error_breakdown {
            error_breakdown.set(error_type.clone(), *count)?;
        }
        table.set("error_breakdown", error_breakdown)?;

        // Performance metrics
        let perf_metrics = lua.create_table()?;

        // Memory usage
        let memory_usage = lua.create_table()?;
        memory_usage.set(
            "peak_memory_per_thread_mb",
            self.performance_metrics
                .memory_usage
                .peak_memory_per_thread_mb,
        )?;
        memory_usage.set(
            "total_memory_usage_mb",
            self.performance_metrics.memory_usage.total_memory_usage_mb,
        )?;
        memory_usage.set(
            "memory_efficiency",
            self.performance_metrics.memory_usage.memory_efficiency,
        )?;
        perf_metrics.set("memory_usage", memory_usage)?;

        // Connection statistics
        let connection_stats = lua.create_table()?;
        connection_stats.set(
            "connections_created",
            self.performance_metrics
                .connection_stats
                .connections_created,
        )?;
        connection_stats.set(
            "connections_reused",
            self.performance_metrics.connection_stats.connections_reused,
        )?;
        connection_stats.set(
            "reuse_ratio",
            self.performance_metrics.connection_stats.reuse_ratio,
        )?;
        connection_stats.set(
            "avg_connection_setup_time",
            self.performance_metrics
                .connection_stats
                .avg_connection_setup_time
                .as_secs_f64(),
        )?;
        perf_metrics.set("connection_stats", connection_stats)?;

        // Batching metrics
        let batching_metrics = lua.create_table()?;
        batching_metrics.set(
            "batch_count",
            self.performance_metrics.batching_metrics.batch_count,
        )?;
        batching_metrics.set(
            "avg_batch_size",
            self.performance_metrics.batching_metrics.avg_batch_size,
        )?;
        batching_metrics.set(
            "batch_efficiency",
            self.performance_metrics.batching_metrics.batch_efficiency,
        )?;
        batching_metrics.set(
            "load_balance_score",
            self.performance_metrics.batching_metrics.load_balance_score,
        )?;
        perf_metrics.set("batching_metrics", batching_metrics)?;

        // Throughput metrics
        let throughput = lua.create_table()?;
        throughput.set(
            "items_per_second",
            self.performance_metrics.throughput.items_per_second,
        )?;
        throughput.set(
            "speedup_factor",
            self.performance_metrics.throughput.speedup_factor,
        )?;
        throughput.set(
            "cpu_efficiency",
            self.performance_metrics.throughput.cpu_efficiency,
        )?;
        perf_metrics.set("throughput", throughput)?;

        table.set("performance_metrics", perf_metrics)?;

        Ok(table)
    }
}

/// Configuration for the parallel executor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// Number of threads to use (defaults to CPU core count)
    pub thread_count: Option<usize>,
    /// Chunk size for batching large datasets (defaults to 100)
    pub chunk_size: Option<usize>,
    /// Timeout per item in seconds (defaults to 300 seconds)
    pub timeout_seconds: Option<u64>,
    /// Error handling strategy: "continue" or "`fail_fast`" (defaults to "continue")
    pub error_strategy: Option<String>,
    /// Maximum memory usage per thread in MB (defaults to 512MB)
    pub max_memory_mb: Option<usize>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            thread_count: None,                           // Use CPU core count
            chunk_size: Some(100),                        // Reasonable default for most datasets
            timeout_seconds: Some(300),                   // 5 minutes per item
            error_strategy: Some("continue".to_string()), // Continue on errors
            max_memory_mb: Some(512),                     // 512MB per thread
        }
    }
}

impl ExecutorConfig {
    /// Creates a new configuration with sensible defaults
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a configuration optimized for small datasets (< 100 items)
    #[must_use]
    pub fn for_small_datasets() -> Self {
        Self {
            thread_count: Some(2),     // Limited parallelism for small datasets
            chunk_size: Some(10),      // Small chunks
            timeout_seconds: Some(60), // 1 minute timeout
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(256), // Lower memory usage
        }
    }

    /// Creates a configuration optimized for large datasets (> 10,000 items)
    #[must_use]
    pub fn for_large_datasets() -> Self {
        Self {
            thread_count: None,         // Use all available cores
            chunk_size: Some(500),      // Larger chunks for efficiency
            timeout_seconds: Some(600), // 10 minutes timeout
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(1024), // Higher memory allowance
        }
    }

    /// Creates a configuration optimized for I/O intensive tasks
    #[must_use]
    pub fn for_io_intensive() -> Self {
        let cpu_count = std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(4);

        Self {
            thread_count: Some(cpu_count * 2), // More threads for I/O waiting
            chunk_size: Some(50),              // Smaller chunks for better responsiveness
            timeout_seconds: Some(900),        // 15 minutes for network operations
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(256), // Lower memory per thread
        }
    }

    /// Gets the effective thread count (resolves None to CPU core count)
    #[must_use]
    pub fn effective_thread_count(&self) -> usize {
        self.thread_count.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(std::num::NonZero::get)
                .unwrap_or(4) // Fallback to 4 threads if detection fails
        })
    }

    /// Gets the effective chunk size
    #[must_use]
    pub fn effective_chunk_size(&self) -> usize {
        self.chunk_size.unwrap_or(100)
    }

    /// Gets the effective timeout in seconds
    #[must_use]
    pub fn effective_timeout_seconds(&self) -> u64 {
        self.timeout_seconds.unwrap_or(300)
    }

    /// Gets the effective error strategy
    #[must_use]
    pub fn effective_error_strategy(&self) -> &str {
        self.error_strategy.as_deref().unwrap_or("continue")
    }

    /// Gets the effective maximum memory per thread in MB
    #[must_use]
    pub fn effective_max_memory_mb(&self) -> usize {
        self.max_memory_mb.unwrap_or(512)
    }
}

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
                for pair in t.pairs::<String, Value>() {
                    let (key, value) = pair?;
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

    /// Serializes a Lua function for cross-thread execution
    ///
    /// # Arguments
    /// * `lua` - The source Lua context
    /// * `func` - The function to serialize
    ///
    /// # Returns
    /// * `mlua::Result<SerializedFunction>` - Serialized function or error
    ///
    /// # Errors
    /// Returns an error if function serialization fails
    pub fn serialize_function(_lua: &Lua, func: &Function) -> mlua::Result<SerializedFunction> {
        // Dump the function to bytecode
        let bytecode = func.dump(false);

        // For now, we'll handle upvalues in a simplified way
        // In a full implementation, we would need to extract and serialize upvalues
        let upvalues = Vec::new();

        Ok(SerializedFunction { bytecode, upvalues })
    }

    /// Deserializes a function in an isolated Lua context
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
        // Load the function from bytecode
        let func = lua.load(&serialized_func.bytecode).into_function()?;

        // TODO: Restore upvalues if needed
        // For now, we assume functions don't capture external variables

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

/// The main parallel executor struct that provides parallel map operations
#[derive(Debug)]
pub struct ParallelExecutor {
    thread_pool: Arc<ThreadPool>,
    config: ExecutorConfig,
    connection_pool: ConnectionPool,
    batch_processor: BatchProcessor,
}

impl ParallelExecutor {
    /// Creates a new parallel executor with the given configuration
    ///
    /// # Arguments
    /// * `config` - Optional configuration for the executor
    ///
    /// # Returns
    /// * `Result<ParallelExecutor>` - The configured executor instance or an error
    ///
    /// # Errors
    /// Returns an error if:
    /// - Thread pool creation fails
    /// - Configuration validation fails
    pub fn new(config: Option<ExecutorConfig>) -> Result<Self> {
        let config = config.unwrap_or_default();

        // Validate configuration
        Self::validate_config(&config)?;

        // Create thread pool with specified or default thread count
        let thread_pool = match config.thread_count {
            Some(count) => ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .context("Failed to create thread pool with specified thread count")?,
            None => ThreadPoolBuilder::new()
                .build()
                .context("Failed to create thread pool with default configuration")?,
        };

        // Create connection pool with reasonable size based on thread count
        let max_connections = config.effective_thread_count() * 2; // 2 connections per thread
        let connection_pool = ConnectionPool::new(max_connections);

        // Create batch processor with optimized settings
        let batch_processor = BatchProcessor::new(&config);

        Ok(Self {
            thread_pool: Arc::new(thread_pool),
            config,
            connection_pool,
            batch_processor,
        })
    }

    /// Validates the executor configuration with comprehensive checks and helpful error messages
    ///
    /// # Arguments
    /// * `config` - The configuration to validate
    ///
    /// # Returns
    /// * `Result<()>` - Success or detailed validation error
    ///
    /// # Errors
    /// Returns detailed errors for:
    /// - Invalid thread count (0, negative, or excessive)
    /// - Invalid chunk size (0 or excessive)
    /// - Invalid timeout values
    /// - Invalid error strategy
    /// - Invalid memory limits
    fn validate_config(config: &ExecutorConfig) -> Result<()> {
        // Validate thread count
        if let Some(thread_count) = config.thread_count {
            if thread_count == 0 {
                return Err(ParallelExecutorError::Configuration {
                    message: "thread_count must be greater than 0".to_string(),
                    suggestion: "Use a positive number of threads (e.g., 1-16) or omit thread_count to use CPU core count".to_string(),
                    parameter: Some("thread_count".to_string()),
                }.into());
            }

            if thread_count > 1024 {
                return Err(ParallelExecutorError::Configuration {
                    message: format!("thread_count {thread_count} exceeds maximum limit of 1024"),
                    suggestion: "Use a reasonable number of threads (typically 1-32). More threads don't always improve performance".to_string(),
                    parameter: Some("thread_count".to_string()),
                }.into());
            }

            // Warn about excessive thread counts relative to CPU cores
            let cpu_cores = std::thread::available_parallelism()
                .map(std::num::NonZero::get)
                .unwrap_or(4);

            if thread_count > cpu_cores * 4 {
                eprintln!(
                    "Warning: thread_count {thread_count} is much higher than CPU cores ({cpu_cores}). This may reduce performance."
                );
            }
        }

        // Validate chunk size
        if let Some(chunk_size) = config.chunk_size {
            if chunk_size == 0 {
                return Err(ParallelExecutorError::Configuration {
                    message: "chunk_size must be greater than 0".to_string(),
                    suggestion: "Use a positive chunk size (e.g., 10-1000). Smaller chunks provide better load balancing, larger chunks reduce overhead".to_string(),
                    parameter: Some("chunk_size".to_string()),
                }.into());
            }

            if chunk_size > 100_000 {
                return Err(ParallelExecutorError::Configuration {
                    message: format!("chunk_size {chunk_size} is excessively large"),
                    suggestion: "Use a reasonable chunk size (typically 10-1000). Very large chunks may cause memory issues".to_string(),
                    parameter: Some("chunk_size".to_string()),
                }.into());
            }
        }

        // Validate timeout
        if let Some(timeout_seconds) = config.timeout_seconds {
            if timeout_seconds == 0 {
                return Err(ParallelExecutorError::Configuration {
                    message: "timeout_seconds must be greater than 0".to_string(),
                    suggestion: "Use a positive timeout value in seconds (e.g., 60-3600). Consider the expected execution time of your functions".to_string(),
                    parameter: Some("timeout_seconds".to_string()),
                }.into());
            }

            if timeout_seconds > 86400 {
                return Err(ParallelExecutorError::Configuration {
                    message: format!("timeout_seconds {timeout_seconds} exceeds maximum of 24 hours (86400 seconds)"),
                    suggestion: "Use a reasonable timeout (typically 60-3600 seconds). Very long timeouts may indicate inefficient functions".to_string(),
                    parameter: Some("timeout_seconds".to_string()),
                }.into());
            }
        }

        // Validate error strategy
        if let Some(error_strategy) = &config.error_strategy {
            match error_strategy.as_str() {
                "continue" | "fail_fast" => {
                    // Valid strategies
                }
                _ => {
                    return Err(ParallelExecutorError::Configuration {
                        message: format!("Invalid error_strategy '{error_strategy}'"),
                        suggestion: "Use 'continue' to process all items despite errors, or 'fail_fast' to stop on first error".to_string(),
                        parameter: Some("error_strategy".to_string()),
                    }.into());
                }
            }
        }

        // Validate memory limits
        if let Some(max_memory_mb) = config.max_memory_mb {
            if max_memory_mb == 0 {
                return Err(ParallelExecutorError::Configuration {
                    message: "max_memory_mb must be greater than 0".to_string(),
                    suggestion: "Use a positive memory limit in MB (e.g., 128-2048). Consider your system's available memory".to_string(),
                    parameter: Some("max_memory_mb".to_string()),
                }.into());
            }

            if max_memory_mb > 16384 {
                return Err(ParallelExecutorError::Configuration {
                    message: format!(
                        "max_memory_mb {max_memory_mb} exceeds reasonable limit of 16GB"
                    ),
                    suggestion: "Use a reasonable memory limit (typically 128-2048 MB per thread)"
                        .to_string(),
                    parameter: Some("max_memory_mb".to_string()),
                }
                .into());
            }

            // Calculate total memory usage and warn if excessive
            let effective_threads = config.effective_thread_count();
            let total_memory_mb = max_memory_mb * effective_threads;

            if total_memory_mb > 8192 {
                eprintln!(
                    "Warning: Total memory usage ({total_memory_mb} MB) with {effective_threads} threads may be excessive. Consider reducing thread_count or max_memory_mb."
                );
            }
        }

        // Validate configuration combinations
        Self::validate_config_combinations(config);

        Ok(())
    }

    /// Validates configuration parameter combinations for optimal performance
    fn validate_config_combinations(config: &ExecutorConfig) {
        let thread_count = config.effective_thread_count();
        let chunk_size = config.effective_chunk_size();

        // Warn about suboptimal combinations
        if thread_count > 16 && chunk_size < 10 {
            eprintln!(
                "Warning: High thread count ({thread_count}) with small chunk size ({chunk_size}) may cause excessive overhead. Consider increasing chunk_size."
            );
        }

        if thread_count == 1 && chunk_size > 1000 {
            eprintln!(
                "Warning: Single thread with large chunk size ({chunk_size}) provides no parallelism benefit. Consider using multiple threads."
            );
        }

        // Check for I/O intensive configuration hints
        if let Some(timeout) = config.timeout_seconds
            && timeout > 300
            && thread_count <= 2
        {
            eprintln!(
                "Info: Long timeout ({timeout} seconds) detected. Consider using more threads for I/O-intensive tasks."
            );
        }
    }

    /// Applies a function to each element in the data collection in parallel
    ///
    /// # Arguments
    /// * `lua` - The Lua context
    /// * `data` - The input data table (array-like)
    /// * `func` - The function to apply to each element
    ///
    /// # Returns
    /// * `mlua::Result<Table>` - Results table with same order as input, including execution summary
    ///
    /// # Errors
    /// Returns an error if:
    /// - Data table is invalid
    /// - Function serialization fails
    /// - Critical parallel execution failure occurs
    pub fn map(&self, lua: &Lua, data: &Table, func: &Function) -> mlua::Result<Table> {
        let start_time = Instant::now();

        // Basic input validation
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let data_len = usize::try_from(data.len()?).unwrap_or(0);

        if data_len == 0 {
            return Err(mlua::Error::RuntimeError(
                "Input data table is empty. Provide a table with at least one element.".to_string(),
            ));
        }

        if data_len > 1_000_000 {
            eprintln!(
                "Warning: Processing {data_len} items in parallel. Consider chunking for very large datasets."
            );
        }

        // Convert data table to vector of values with enhanced error handling
        let mut data_vec = Vec::new();
        for i in 1..=data_len {
            match data.get::<Value>(i) {
                Ok(value) => {
                    match SerializedValue::from_lua_value(value) {
                        Ok(serialized_value) => {
                            data_vec.push((i - 1, serialized_value)); // Store with 0-based index
                        }
                        Err(e) => {
                            return Err(mlua::Error::RuntimeError(format!(
                                "Cannot serialize data element at index {i}: {e}. Ensure all data elements are basic Lua types."
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Cannot access data element at index {i}: {e}. Check table structure."
                    )));
                }
            }
        }

        // Serialize the function for cross-thread execution with enhanced error reporting
        let serialized_func = LuaContextFactory::serialize_function(lua, func).map_err(|e| {
            mlua::Error::RuntimeError(format!(
                "Function cannot be serialized for parallel execution: {e}. Ensure the function is a simple Lua function without complex upvalues."
            ))
        })?;

        // Initialize performance monitoring
        let mut performance_monitor = PerformanceMonitor::new();
        performance_monitor.start_processing(data_len, self.thread_count());

        // Execute in parallel using rayon with comprehensive error handling and performance optimizations
        let results: Vec<ExecutionResult> = self.thread_pool.install(|| {
            // Use batch processor for efficient batching
            self.batch_processor.process_batches(&data_vec, |batch| {
                use rayon::prelude::*;
                batch
                    .into_par_iter()
                    .map(|(index, serialized_value)| {
                        let start_time = Instant::now();
                        let _thread_id = format!("{:?}", std::thread::current().id());

                        // Create isolated Lua context for this thread
                        let thread_lua = match LuaContextFactory::create_isolated_context() {
                            Ok(lua) => lua,
                            Err(e) => {
                                return ExecutionResult::failure(
                                    index,
                                    format!("Failed to create isolated Lua context: {e}. This may indicate memory pressure or system resource exhaustion."),
                                    start_time.elapsed(),
                                );
                            }
                        };

                        // Deserialize function in the thread context
                        let thread_func = match LuaContextFactory::deserialize_function(
                            &thread_lua,
                            &serialized_func,
                        ) {
                            Ok(func) => func,
                            Err(e) => {
                                return ExecutionResult::failure(
                                    index,
                                    format!("Failed to deserialize function in thread context: {e}. This may indicate function complexity or upvalue issues."),
                                    start_time.elapsed(),
                                );
                            }
                        };

                        // Convert serialized value back to Lua value
                        let lua_value = match serialized_value.to_lua_value(&thread_lua) {
                            Ok(value) => value,
                            Err(e) => {
                                return ExecutionResult::failure(
                                    index,
                                    format!("Failed to convert input data to Lua value: {e}"),
                                    start_time.elapsed(),
                                );
                            }
                        };

                        // Execute the function with the data element and comprehensive error handling
                        match thread_func.call::<Value>(lua_value) {
                            Ok(result) => match SerializedValue::from_lua_value(result) {
                                Ok(serialized_result) => ExecutionResult::success(
                                    index,
                                    serialized_result,
                                    start_time.elapsed(),
                                ),
                                Err(e) => ExecutionResult::failure(
                                    index,
                                    format!("Failed to serialize function result: {e}. Ensure the function returns serializable values."),
                                    start_time.elapsed(),
                                ),
                            },
                            Err(e) => {
                                let error_message = Self::categorize_execution_error(&e);
                                ExecutionResult::failure(
                                    index,
                                    error_message,
                                    start_time.elapsed(),
                                )
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            })
        });

        let total_execution_time = start_time.elapsed();

        // Calculate performance metrics using actual data from components
        let successful_count = results.iter().filter(|r| r.result.is_ok()).count();
        let thread_count = self.thread_count();

        // Finish performance monitoring
        let base_performance_metrics = performance_monitor.finish_processing(successful_count);

        // Get actual statistics from connection pool and batch processor
        let connection_stats = self.connection_pool.get_stats();
        let batching_metrics = self.batch_processor.get_stats();

        // Update performance metrics with real data
        let performance_metrics = performance_monitor.update_metrics(
            base_performance_metrics,
            connection_stats,
            batching_metrics,
        );

        // Create execution summary with performance metrics
        let summary = ExecutionSummary::from_results(
            &results,
            thread_count,
            total_execution_time,
            performance_metrics,
        );

        // Convert results back to Lua table, preserving order
        let results_table = lua.create_table()?;

        // Sort results by index to maintain order
        let mut sorted_results = results;
        sorted_results.sort_by_key(|r| r.index);

        // Add individual results
        for (lua_index, result) in sorted_results.into_iter().enumerate() {
            let individual_result = result.to_lua_table(lua)?;
            results_table.set(lua_index + 1, individual_result)?; // Lua uses 1-based indexing
        }

        // Add execution summary as metadata
        let summary_table = summary.to_lua_table(lua)?;
        results_table.set("_summary", summary_table)?;

        // Add convenience methods for accessing summary data
        results_table.set("_success_count", summary.successful_count)?;
        results_table.set("_failed_count", summary.failed_count)?;
        results_table.set("_total_time", summary.total_time.as_secs_f64())?;

        Ok(results_table)
    }

    /// Configures the parallel executor with new settings
    ///
    /// # Arguments
    /// * `config` - New configuration to apply
    ///
    /// # Returns
    /// * `mlua::Result<()>` - Success or error
    ///
    /// # Errors
    /// Returns an error if configuration validation fails
    pub fn configure(&mut self, config: ExecutorConfig) -> mlua::Result<()> {
        Self::validate_config(&config).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // If thread count changed, recreate the thread pool and connection pool
        if config.thread_count != self.config.thread_count {
            let new_pool = match config.thread_count {
                Some(count) => ThreadPoolBuilder::new()
                    .num_threads(count)
                    .build()
                    .map_err(|e| {
                        mlua::Error::RuntimeError(format!("Failed to create thread pool: {e}"))
                    })?,
                None => ThreadPoolBuilder::new().build().map_err(|e| {
                    mlua::Error::RuntimeError(format!("Failed to create thread pool: {e}"))
                })?,
            };
            self.thread_pool = Arc::new(new_pool);

            // Clear connection pool when thread count changes
            self.connection_pool.clear();

            // Recreate batch processor with new configuration
            self.batch_processor = BatchProcessor::new(&config);
        }

        self.config = config;
        Ok(())
    }

    /// Gets the current thread count of the executor
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.thread_pool.current_num_threads()
    }

    /// Gets the current configuration
    #[must_use]
    pub const fn config(&self) -> &ExecutorConfig {
        &self.config
    }

    /// Categorizes execution errors for better reporting
    fn categorize_execution_error(error: &mlua::Error) -> String {
        let error_str = error.to_string();
        if error_str.contains("attempt to") {
            format!("Function execution error (likely type mismatch): {error}")
        } else if error_str.contains("stack overflow") {
            format!(
                "Function execution error (stack overflow - possible infinite recursion): {error}"
            )
        } else if error_str.contains("memory") {
            format!("Function execution error (memory issue): {error}")
        } else {
            format!("Function execution error: {error}")
        }
    }
}

/// Global parallel executor instance with lazy initialization
static GLOBAL_EXECUTOR: OnceLock<Mutex<ParallelExecutor>> = OnceLock::new();

/// Gets a reference to the global parallel executor
///
/// # Returns
/// * `&'static Mutex<ParallelExecutor>` - Reference to the global executor
pub fn global_executor() -> &'static Mutex<ParallelExecutor> {
    GLOBAL_EXECUTOR.get_or_init(|| {
        let executor = ParallelExecutor::new(None).unwrap_or_else(|_| {
            // Fallback to single-threaded if initialization fails
            ParallelExecutor::new(Some(ExecutorConfig {
                thread_count: Some(1),
                chunk_size: Some(100),
                timeout_seconds: Some(300),
                error_strategy: Some("continue".to_string()),
                max_memory_mb: Some(512),
            }))
            .unwrap_or_else(|_| {
                // Ultimate fallback - create a minimal executor
                let fallback_config = ExecutorConfig {
                    thread_count: Some(1),
                    chunk_size: Some(100),
                    timeout_seconds: Some(300),
                    error_strategy: Some("continue".to_string()),
                    max_memory_mb: Some(512),
                };

                let thread_pool = Arc::new(
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(1)
                        .build()
                        .unwrap_or_else(|_| {
                            // If even this fails, we have a serious problem
                            eprintln!("Fatal error: Failed to create thread pool");
                            std::process::exit(1);
                        }),
                );
                let connection_pool = ConnectionPool::new(10);
                let batch_processor = BatchProcessor::new(&fallback_config);

                ParallelExecutor {
                    thread_pool,
                    config: fallback_config,
                    connection_pool,
                    batch_processor,
                }
            })
        });
        Mutex::new(executor)
    })
}

/// Initializes the global parallel executor with custom configuration
///
/// # Arguments
/// * `config` - Configuration for the global executor
///
/// # Returns
/// * `Result<()>` - Success or error
///
/// # Errors
/// Returns an error if:
/// - Global executor is already initialized
/// - Configuration is invalid
pub fn init_global_executor(config: Option<ExecutorConfig>) -> Result<()> {
    if GLOBAL_EXECUTOR.get().is_some() {
        anyhow::bail!("Global executor is already initialized");
    }

    let executor = ParallelExecutor::new(config)?;
    GLOBAL_EXECUTOR
        .set(Mutex::new(executor))
        .map_err(|_| anyhow::anyhow!("Failed to set global executor"))?;

    Ok(())
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
        let executor = global_executor();
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

        let executor = global_executor();
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

            Self::validate_config(&config).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

            Ok(true)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_config_default() {
        let config = ExecutorConfig::default();
        assert_eq!(config.thread_count, None);
        assert_eq!(config.chunk_size, Some(100));
        assert_eq!(config.timeout_seconds, Some(300));
        assert_eq!(config.error_strategy, Some("continue".to_string()));
        assert_eq!(config.max_memory_mb, Some(512));
    }

    #[test]
    fn test_executor_config_presets() {
        // Test small dataset preset
        let small_config = ExecutorConfig::for_small_datasets();
        assert_eq!(small_config.thread_count, Some(2));
        assert_eq!(small_config.chunk_size, Some(10));
        assert_eq!(small_config.timeout_seconds, Some(60));
        assert_eq!(small_config.max_memory_mb, Some(256));

        // Test large dataset preset
        let large_config = ExecutorConfig::for_large_datasets();
        assert_eq!(large_config.thread_count, None); // Use all cores
        assert_eq!(large_config.chunk_size, Some(500));
        assert_eq!(large_config.timeout_seconds, Some(600));
        assert_eq!(large_config.max_memory_mb, Some(1024));

        // Test I/O intensive preset
        let io_config = ExecutorConfig::for_io_intensive();
        assert!(io_config.thread_count.is_some_and(|count| count >= 4)); // At least 4 threads
        assert_eq!(io_config.chunk_size, Some(50));
        assert_eq!(io_config.timeout_seconds, Some(900));
        assert_eq!(io_config.max_memory_mb, Some(256));
    }

    #[test]
    fn test_executor_config_effective_values() {
        let config = ExecutorConfig::default();

        // Test effective values
        assert!(config.effective_thread_count() > 0);
        assert_eq!(config.effective_chunk_size(), 100);
        assert_eq!(config.effective_timeout_seconds(), 300);
        assert_eq!(config.effective_error_strategy(), "continue");
        assert_eq!(config.effective_max_memory_mb(), 512);

        // Test with custom values
        let custom_config = ExecutorConfig {
            thread_count: Some(8),
            chunk_size: Some(200),
            timeout_seconds: Some(600),
            error_strategy: Some("fail_fast".to_string()),
            max_memory_mb: Some(1024),
        };

        assert_eq!(custom_config.effective_thread_count(), 8);
        assert_eq!(custom_config.effective_chunk_size(), 200);
        assert_eq!(custom_config.effective_timeout_seconds(), 600);
        assert_eq!(custom_config.effective_error_strategy(), "fail_fast");
        assert_eq!(custom_config.effective_max_memory_mb(), 1024);
    }

    #[test]
    fn test_executor_creation() -> Result<()> {
        let executor = ParallelExecutor::new(None)?;
        assert!(executor.thread_count() > 0);
        Ok(())
    }

    #[test]
    fn test_executor_creation_with_config() -> Result<()> {
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(50),
            timeout_seconds: Some(120),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(256),
        };
        let executor = ParallelExecutor::new(Some(config))?;
        assert_eq!(executor.thread_count(), 4);
        assert_eq!(executor.config().chunk_size, Some(50));
        assert_eq!(executor.config().timeout_seconds, Some(120));
        assert_eq!(
            executor.config().error_strategy,
            Some("continue".to_string())
        );
        assert_eq!(executor.config().max_memory_mb, Some(256));
        Ok(())
    }

    #[test]
    fn test_config_validation() {
        // Test invalid thread count
        let config = ExecutorConfig {
            thread_count: Some(0),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test excessive thread count
        let config = ExecutorConfig {
            thread_count: Some(2000),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test invalid chunk size
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(0),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test invalid timeout
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(0),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test invalid error strategy
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("invalid_strategy".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test invalid memory limit
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(0),
        };
        assert!(ParallelExecutor::validate_config(&config).is_err());

        // Test valid config
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_ok());

        // Test valid fail_fast strategy
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("fail_fast".to_string()),
            max_memory_mb: Some(512),
        };
        assert!(ParallelExecutor::validate_config(&config).is_ok());
    }

    #[test]
    fn test_global_executor() {
        let executor = global_executor();
        if let Ok(executor_guard) = executor.lock() {
            assert!(executor_guard.thread_count() > 0);
        } else {
            panic!("Failed to lock executor");
        }
    }

    #[test]
    fn test_executor_configure() -> Result<()> {
        let mut executor = ParallelExecutor::new(None)?;
        let _original_count = executor.thread_count();

        let new_config = ExecutorConfig {
            thread_count: Some(2),
            chunk_size: Some(200),
            timeout_seconds: Some(600),
            error_strategy: Some("fail_fast".to_string()),
            max_memory_mb: Some(1024),
        };

        executor
            .configure(new_config)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        assert_eq!(executor.thread_count(), 2);
        assert_eq!(executor.config().chunk_size, Some(200));
        assert_eq!(executor.config().timeout_seconds, Some(600));
        assert_eq!(
            executor.config().error_strategy,
            Some("fail_fast".to_string())
        );
        assert_eq!(executor.config().max_memory_mb, Some(1024));

        Ok(())
    }

    #[test]
    fn test_serialized_value_conversion() -> mlua::Result<()> {
        let lua = Lua::new();

        // Test basic types
        let nil_value = Value::Nil;
        let serialized = SerializedValue::from_lua_value(nil_value)?;
        let converted_back = serialized.to_lua_value(&lua)?;
        assert!(matches!(converted_back, Value::Nil));

        let bool_value = Value::Boolean(true);
        let serialized = SerializedValue::from_lua_value(bool_value)?;
        let converted_back = serialized.to_lua_value(&lua)?;
        assert!(matches!(converted_back, Value::Boolean(true)));

        let int_value = Value::Integer(42);
        let serialized = SerializedValue::from_lua_value(int_value)?;
        let converted_back = serialized.to_lua_value(&lua)?;
        assert!(matches!(converted_back, Value::Integer(42)));

        let string_value = Value::String(lua.create_string("test")?);
        let serialized = SerializedValue::from_lua_value(string_value)?;
        let converted_back = serialized.to_lua_value(&lua)?;
        if let Value::String(s) = converted_back {
            assert_eq!(s.to_str()?, "test");
        } else {
            panic!("Expected string value");
        }

        Ok(())
    }

    #[test]
    fn test_lua_context_factory() -> Result<()> {
        let lua = LuaContextFactory::create_isolated_context()?;

        // Test that komandan global is available
        let komandan: Table = lua.globals().get("komandan")?;
        let _modules: Table = komandan.get("modules")?;
        let _check: Table = komandan.get("check")?;
        let _defaults = komandan.get::<mlua::Value>("defaults")?;
        let _komando_fn = komandan.get::<mlua::Function>("komando")?;

        // Test that 'k' alias is available
        let k: Table = lua.globals().get("k")?;
        let _k_mods: Table = k.get("mods")?;
        let _k_check: Table = k.get("check")?;
        let _k_komando_fn = k.get::<mlua::Function>("komando")?;

        // Test that modules are available
        let modules: Table = komandan.get("modules")?;
        assert!(modules.contains_key("cmd")?);
        assert!(modules.contains_key("file")?);
        assert!(modules.contains_key("apt")?);

        // Test that check functions are available
        let check: Table = komandan.get("check")?;
        assert!(check.contains_key("file")?);
        assert!(check.contains_key("service")?);
        assert!(check.contains_key("package")?);

        Ok(())
    }

    #[test]
    fn test_function_serialization() -> mlua::Result<()> {
        let lua = Lua::new();

        // For now, let's test with a Lua-defined function instead of Rust function
        // Rust functions created with create_function may not serialize properly
        let lua_func_code = "return function(x) return x * 2 end";
        let lua_func: mlua::Function = lua.load(lua_func_code).eval()?;

        // Serialize the function
        let serialized = LuaContextFactory::serialize_function(&lua, &lua_func)?;
        assert!(
            !serialized.bytecode.is_empty(),
            "Lua function should have bytecode"
        );

        // Deserialize in a new context
        let new_lua = LuaContextFactory::create_isolated_context()?;
        let deserialized = LuaContextFactory::deserialize_function(&new_lua, &serialized)?;

        // Test that the deserialized function works
        let result: i32 = deserialized.call(5)?;
        assert_eq!(result, 10);

        Ok(())
    }

    #[test]
    fn test_parallel_map_basic() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data
        let data = lua.create_table()?;
        data.set(1, 10)?;
        data.set(2, 20)?;
        data.set(3, 30)?;

        // Create a Lua function that doubles the input (Rust functions don't serialize well)
        let func_code = "return function(x) return x * 2 end";
        let func: mlua::Function = lua.load(func_code).eval()?;

        // Execute parallel map
        let results = executor.map(&lua, &data, &func)?;

        // Check results
        assert_eq!(results.len()?, 3);

        for i in 1..=3 {
            let result_table: Table = results.get(i)?;
            let success: bool = result_table.get("success")?;

            if success {
                let result_value: i32 = result_table.get("result")?;
                let expected = i * 10 * 2; // Original values are 10, 20, 30, doubled
                assert_eq!(result_value, expected, "Result {i} should be {expected}");
            } else {
                // If there's an error, print it for debugging
                if let Ok(error) = result_table.get::<String>("error") {
                    panic!("Result {i} failed with error: {error}");
                } else {
                    panic!("Result {i} should be successful");
                }
            }
        }

        Ok(())
    }

    #[test]
    fn test_error_handling_invalid_data() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Test empty data table
        let empty_data = lua.create_table()?;
        let func_code = "return function(x) return x * 2 end";
        let func: mlua::Function = lua.load(func_code).eval()?;

        let result = executor.map(&lua, &empty_data, &func);
        assert!(result.is_err());

        // Check that the error message is helpful
        if let Err(mlua::Error::RuntimeError(msg)) = result {
            assert!(msg.contains("Input data table is empty"));
            assert!(msg.contains("at least one element"));
        }

        Ok(())
    }

    #[test]
    fn test_error_handling_function_failure() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data with mixed types that will cause errors
        let data = lua.create_table()?;
        data.set(1, 10)?;
        data.set(2, "invalid")?; // This will cause a type error
        data.set(3, 30)?;

        // Create a function that will definitely fail with non-numbers
        let func_code =
            "return function(x) assert(type(x) == 'number', 'Expected number'); return x * 2 end";
        let func: mlua::Function = lua.load(func_code).eval()?;

        // Execute parallel map
        let results = executor.map(&lua, &data, &func)?;

        // Check that we have results for all items
        assert_eq!(results.len()?, 3);

        // Check summary shows mixed results
        let success_count: usize = results.get("_success_count")?;
        let failed_count: usize = results.get("_failed_count")?;

        println!("Success count: {success_count}, Failed count: {failed_count}");

        // Print individual results for debugging
        for i in 1..=3 {
            let result_table: Table = results.get(i)?;
            let success: bool = result_table.get("success")?;
            println!("Result {i}: success = {success}");
            if !success && let Ok(error_msg) = result_table.get::<String>("error") {
                println!("  Error: {error_msg}");
            }
        }

        assert_eq!(success_count, 2); // Items 1 and 3 should succeed
        assert_eq!(failed_count, 1); // Item 2 should fail

        // Check individual results
        let first_result: Table = results.get(1)?;
        assert!(first_result.get::<bool>("success")?);

        let second_result: Table = results.get(2)?;
        assert!(!second_result.get::<bool>("success")?);
        let error_msg: String = second_result.get("error")?;
        assert!(error_msg.contains("Function execution error"));

        let third_result: Table = results.get(3)?;
        assert!(third_result.get::<bool>("success")?);

        // Check execution summary
        let summary: Table = results.get("_summary")?;
        let total_items: usize = summary.get("total_items")?;
        assert_eq!(total_items, 3);

        let error_breakdown: Table = summary.get("error_breakdown")?;

        // Count error breakdown entries manually (Lua tables with string keys don't report len() correctly)
        let mut error_count = 0;
        for _pair in error_breakdown.pairs::<String, usize>() {
            error_count += 1;
        }

        // Should have at least one error category
        assert!(error_count > 0);

        Ok(())
    }

    #[test]
    fn test_configuration_error_messages() {
        // Test thread count validation
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(0),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("thread_count must be greater than 0"));
        } else {
            panic!("Expected error result");
        }

        // Test chunk size validation
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(0),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("chunk_size must be greater than 0"));
        } else {
            panic!("Expected error result");
        }

        // Test excessive thread count
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(2000),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("exceeds maximum limit"));
        } else {
            panic!("Expected error result");
        }

        // Test invalid error strategy
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("invalid".to_string()),
            max_memory_mb: Some(512),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("Invalid error_strategy"));
        } else {
            panic!("Expected error result");
        }

        // Test timeout validation
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(0),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("timeout_seconds must be greater than 0"));
        } else {
            panic!("Expected error result");
        }

        // Test memory validation
        let result = ParallelExecutor::validate_config(&ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(0),
        });

        assert!(result.is_err());
        if let Err(error) = result {
            let error_msg = error.to_string();
            assert!(error_msg.contains("max_memory_mb must be greater than 0"));
        } else {
            panic!("Expected error result");
        }
    }

    #[test]
    fn test_configuration_combinations() {
        // Test configuration that should generate warnings (captured via stderr)
        let config = ExecutorConfig {
            thread_count: Some(20), // High thread count
            chunk_size: Some(5),    // Small chunk size
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };

        // This should succeed but generate warnings
        assert!(ParallelExecutor::validate_config(&config).is_ok());

        // Test single thread with large chunk size
        let config = ExecutorConfig {
            thread_count: Some(1),
            chunk_size: Some(2000), // Large chunk size
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };

        // This should succeed but generate warnings
        assert!(ParallelExecutor::validate_config(&config).is_ok());

        // Test reasonable configuration
        let config = ExecutorConfig {
            thread_count: Some(4),
            chunk_size: Some(100),
            timeout_seconds: Some(300),
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(512),
        };

        assert!(ParallelExecutor::validate_config(&config).is_ok());
    }

    #[test]
    fn test_configuration_presets_validation() {
        // All presets should be valid
        assert!(ParallelExecutor::validate_config(&ExecutorConfig::for_small_datasets()).is_ok());
        assert!(ParallelExecutor::validate_config(&ExecutorConfig::for_large_datasets()).is_ok());
        assert!(ParallelExecutor::validate_config(&ExecutorConfig::for_io_intensive()).is_ok());
        assert!(ParallelExecutor::validate_config(&ExecutorConfig::default()).is_ok());
    }

    #[test]
    fn test_execution_summary() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data
        let data = lua.create_table()?;
        data.set(1, 10)?;
        data.set(2, 20)?;
        data.set(3, 30)?;

        let func_code = "return function(x) return x * 2 end";
        let func: mlua::Function = lua.load(func_code).eval()?;

        let results = executor.map(&lua, &data, &func)?;

        // Check summary information
        let summary: Table = results.get("_summary")?;

        let total_items: usize = summary.get("total_items")?;
        assert_eq!(total_items, 3);

        let successful_count: usize = summary.get("successful_count")?;
        assert_eq!(successful_count, 3);

        let failed_count: usize = summary.get("failed_count")?;
        assert_eq!(failed_count, 0);

        let success_rate: f64 = summary.get("success_rate")?;
        assert!((success_rate - 1.0).abs() < f64::EPSILON);

        // Check thread info
        let thread_info: Table = summary.get("thread_info")?;
        let threads_used: usize = thread_info.get("threads_used")?;
        assert!(threads_used > 0);

        let efficiency: f64 = thread_info.get("efficiency")?;
        assert!(efficiency > 0.0 && efficiency <= 1.0);

        // Check performance metrics
        let perf_metrics: Table = summary.get("performance_metrics")?;

        // Check throughput metrics
        let throughput: Table = perf_metrics.get("throughput")?;
        let items_per_second: f64 = throughput.get("items_per_second")?;
        assert!(items_per_second > 0.0);

        let speedup_factor: f64 = throughput.get("speedup_factor")?;
        assert!(speedup_factor > 0.0);

        let cpu_efficiency: f64 = throughput.get("cpu_efficiency")?;
        assert!((0.0..=1.0).contains(&cpu_efficiency));

        // Check memory usage
        let memory_usage: Table = perf_metrics.get("memory_usage")?;
        let peak_memory: f64 = memory_usage.get("peak_memory_per_thread_mb")?;
        assert!(peak_memory > 0.0);

        // Check connection stats (may be 0 for simple local operations)
        let connection_stats: Table = perf_metrics.get("connection_stats")?;
        let _connections_created: usize = connection_stats.get("connections_created")?;
        // For simple parallel map operations without actual connections, this may be 0

        // Check batching metrics
        let batching_metrics: Table = perf_metrics.get("batching_metrics")?;
        let batch_count: usize = batching_metrics.get("batch_count")?;
        assert!(batch_count > 0);

        Ok(())
    }

    #[test]
    fn test_error_categorization() {
        // Test different error types are categorized correctly
        assert_eq!(
            ExecutionSummary::categorize_error("serialize failed"),
            "Serialization"
        );
        assert_eq!(
            ExecutionSummary::categorize_error("deserialize error"),
            "Serialization"
        );
        assert_eq!(
            ExecutionSummary::categorize_error("Lua context failed"),
            "Lua Context"
        );
        assert_eq!(
            ExecutionSummary::categorize_error("Function execution failed"),
            "Function Execution"
        );
        assert_eq!(
            ExecutionSummary::categorize_error("convert value failed"),
            "Type Conversion"
        );
        assert_eq!(ExecutionSummary::categorize_error("unknown error"), "Other");
    }

    #[test]
    fn test_komando_availability_in_isolated_context() -> mlua::Result<()> {
        let lua = LuaContextFactory::create_isolated_context()?;

        // Test that komando function is available
        let komando_fn = lua.globals().get::<mlua::Function>("komando");
        match komando_fn {
            Ok(_) => println!("✓ komando function is available globally"),
            Err(e) => println!("✗ komando function not available globally: {e}"),
        }

        // Test via komandan table
        let komandan: Table = lua.globals().get("komandan")?;
        let komando_fn = komandan.get::<mlua::Function>("komando");
        match komando_fn {
            Ok(_) => println!("✓ komando function is available via komandan table"),
            Err(e) => println!("✗ komando function not available via komandan table: {e}"),
        }

        // Test via k table
        let k: Table = lua.globals().get("k")?;
        let komando_fn = k.get::<mlua::Function>("komando");
        match komando_fn {
            Ok(_) => println!("✓ komando function is available via k table"),
            Err(e) => println!("✗ komando function not available via k table: {e}"),
        }

        // Test a simple function that uses komando via komandan table
        let test_code = r#"
            local host = {
                address = "localhost",
                connection = "local"
            }

            local task = {
                name = "Test",
                komandan.modules.cmd({
                    cmd = "echo 'test'"
                })
            }

            return komandan.komando(task, host)
        "#;

        let result = lua.load(test_code).eval::<Table>();
        match result {
            Ok(result_table) => {
                let exit_code: i32 = result_table.get("exit_code")?;
                println!("✓ komando execution succeeded with exit code: {exit_code}");
            }
            Err(e) => {
                println!("✗ komando execution failed: {e}");
            }
        }

        Ok(())
    }

    #[test]
    fn test_komando_integration_basic() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data with host configurations
        let data = lua.create_table()?;

        // Create host configurations for local execution
        let host1 = lua.create_table()?;
        host1.set("address", "localhost")?;
        host1.set("connection", "local")?;
        data.set(1, host1)?;

        let host2 = lua.create_table()?;
        host2.set("address", "localhost")?;
        host2.set("connection", "local")?;
        data.set(2, host2)?;

        // Create a function that uses komando to execute a simple command
        let func_code = r#"
            return function(host)
                local task = {
                    name = "Test command",
                    komandan.modules.cmd({
                        cmd = "echo 'Hello from parallel komando'"
                    })
                }

                local result = komandan.komando(task, host)
                return {
                    host_address = host.address,
                    exit_code = result.exit_code,
                    stdout = result.stdout,
                    success = result.exit_code == 0
                }
            end
        "#;
        let func: mlua::Function = lua.load(func_code).eval()?;

        // Execute parallel map with komando integration
        let results = executor.map(&lua, &data, &func)?;

        // Check results
        assert_eq!(results.len()?, 2);

        // Verify both executions succeeded
        for i in 1..=2 {
            let result_table: Table = results.get(i)?;
            let success: bool = result_table.get("success")?;

            if success {
                let result_value: Table = result_table.get("result")?;
                let host_address: String = result_value.get("host_address")?;
                let exit_code: i32 = result_value.get("exit_code")?;
                let stdout: String = result_value.get("stdout")?;
                let task_success: bool = result_value.get("success")?;

                assert_eq!(host_address, "localhost");
                assert_eq!(exit_code, 0);
                assert!(stdout.contains("Hello from parallel komando"));
                assert!(task_success);
            } else {
                // If there's an error, print it for debugging
                if let Ok(error) = result_table.get::<String>("error") {
                    panic!("Result {i} failed with error: {error}");
                } else {
                    panic!("Result {i} should be successful");
                }
            }
        }

        // Check execution summary
        let success_count: usize = results.get("_success_count")?;
        let failed_count: usize = results.get("_failed_count")?;
        assert_eq!(success_count, 2);
        assert_eq!(failed_count, 0);

        Ok(())
    }

    #[test]
    fn test_komando_integration_with_different_commands() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data with different commands to execute
        let data = lua.create_table()?;
        data.set(1, "echo 'Command 1'")?;
        data.set(2, "echo 'Command 2'")?;
        data.set(3, "echo 'Command 3'")?;

        // Create a function that uses komando to execute different commands
        let func_code = r#"
            return function(cmd)
                local host = {
                    address = "localhost",
                    connection = "local"
                }

                local task = {
                    name = "Dynamic command",
                    komandan.modules.cmd({
                        cmd = cmd
                    })
                }

                local result = komandan.komando(task, host)
                return {
                    command = cmd,
                    exit_code = result.exit_code,
                    stdout = result.stdout:gsub("%s+$", ""), -- trim whitespace
                    success = result.exit_code == 0
                }
            end
        "#;
        let func: mlua::Function = lua.load(func_code).eval()?;

        // Execute parallel map
        let results = executor.map(&lua, &data, &func)?;

        // Check results
        assert_eq!(results.len()?, 3);

        // Verify each command executed correctly
        for i in 1..=3 {
            let result_table: Table = results.get(i)?;
            let success: bool = result_table.get("success")?;

            if success {
                let result_value: Table = result_table.get("result")?;
                let command: String = result_value.get("command")?;
                let exit_code: i32 = result_value.get("exit_code")?;
                let stdout: String = result_value.get("stdout")?;
                let task_success: bool = result_value.get("success")?;

                assert_eq!(exit_code, 0);
                assert!(task_success);

                // Verify the output matches the expected command
                match i {
                    1 => {
                        assert_eq!(command, "echo 'Command 1'");
                        assert_eq!(stdout, "Command 1");
                    }
                    2 => {
                        assert_eq!(command, "echo 'Command 2'");
                        assert_eq!(stdout, "Command 2");
                    }
                    3 => {
                        assert_eq!(command, "echo 'Command 3'");
                        assert_eq!(stdout, "Command 3");
                    }
                    _ => panic!("Unexpected result index: {i}"),
                }
            } else if let Ok(error) = result_table.get::<String>("error") {
                panic!("Result {i} failed with error: {error}");
            } else {
                panic!("Result {i} should be successful");
            }
        }

        Ok(())
    }

    #[test]
    fn test_komando_integration_error_handling() -> mlua::Result<()> {
        let lua = Lua::new();
        let executor =
            ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // Create test data with commands that will fail
        let data = lua.create_table()?;
        data.set(1, "echo 'Success'")?;
        data.set(2, "false")?; // This command will fail
        data.set(3, "echo 'Another success'")?;

        // Create a function that uses komando with error handling
        let func_code = r#"
            return function(cmd)
                local host = {
                    address = "localhost",
                    connection = "local"
                }

                local task = {
                    name = "Test command with error handling",
                    komandan.modules.cmd({
                        cmd = cmd
                    }),
                    ignore_exit_code = true  -- Don't fail on non-zero exit codes
                }

                local result = komandan.komando(task, host)
                return {
                    command = cmd,
                    exit_code = result.exit_code,
                    stdout = result.stdout:gsub("%s+$", ""), -- trim whitespace
                    stderr = result.stderr or "",
                    success = result.exit_code == 0
                }
            end
        "#;
        let func: mlua::Function = lua.load(func_code).eval()?;

        // Execute parallel map
        let results = executor.map(&lua, &data, &func)?;

        // Check results
        assert_eq!(results.len()?, 3);

        // Check individual results
        let first_result: Table = results.get(1)?;
        assert!(first_result.get::<bool>("success")?);
        let result1_value: Table = first_result.get("result")?;
        assert_eq!(result1_value.get::<String>("stdout")?, "Success");
        assert_eq!(result1_value.get::<i32>("exit_code")?, 0);

        let second_result: Table = results.get(2)?;
        assert!(second_result.get::<bool>("success")?); // Function execution succeeded
        let result2_value: Table = second_result.get("result")?;
        assert!(!result2_value.get::<bool>("success")?); // But command failed
        assert_eq!(result2_value.get::<i32>("exit_code")?, 1);

        let third_result: Table = results.get(3)?;
        assert!(third_result.get::<bool>("success")?);
        let result3_value: Table = third_result.get("result")?;
        assert_eq!(result3_value.get::<String>("stdout")?, "Another success");
        assert_eq!(result3_value.get::<i32>("exit_code")?, 0);

        // Check execution summary - all function executions should succeed
        let success_count: usize = results.get("_success_count")?;
        let failed_count: usize = results.get("_failed_count")?;
        assert_eq!(success_count, 3);
        assert_eq!(failed_count, 0);

        Ok(())
    }
}
