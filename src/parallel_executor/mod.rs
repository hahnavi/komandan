mod batch;
mod config;
mod error;
mod lua_bridge;
mod monitor;
mod pool;
mod summary;
mod validation;

#[cfg(test)]
mod tests;

pub use error::ParallelExecutorError;
pub use lua_bridge::{create_global_executor_interface, parallel_executor_constructor};

pub use batch::BatchProcessor;
pub use config::ExecutorConfig;
pub use lua_bridge::{ExecutionResult, LuaContextFactory, SerializedFunction, SerializedValue};
pub use monitor::PerformanceMonitor;
pub use pool::ConnectionPool;
pub use summary::{
    BatchingMetrics, ConnectionStats, ExecutionSummary, MemoryUsage, PerformanceMetrics,
    ThreadUtilization, ThroughputMetrics,
};
pub(crate) use validation::validate_config;

use anyhow::{Context, Result};
use mlua::{Function, Lua, Table, Value};
use rayon::{ThreadPool, ThreadPoolBuilder};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
        validate_config(&config)?;

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
            tracing::warn!(
                "Processing {data_len} items in parallel. Consider chunking for very large datasets."
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

        // Sum of per-item execution times — the wall-clock work that would
        // have run serially. Used by the throughput calculator to derive a
        // real speedup factor instead of an algebraic constant.
        let sequential_work: Duration = results.iter().map(|r| r.execution_time).sum::<Duration>();

        // Finish performance monitoring
        let base_performance_metrics =
            performance_monitor.finish_processing(successful_count, sequential_work);

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
        validate_config(&config).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

        // If thread count changed, recreate the thread pool and connection pool.
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
        }

        // Rebuild the batch processor whenever ANY of its inputs change
        // (chunk_size, thread_count-derived sizing, memory threshold), not
        // just on thread_count changes. Otherwise `map()` keeps using the old
        // optimal_batch_size even after `configure` updated `self.config`.
        if config.chunk_size != self.config.chunk_size
            || config.thread_count != self.config.thread_count
            || config.max_memory_mb != self.config.max_memory_mb
        {
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

/// Global parallel executor instance.
///
/// Initialized explicitly via [`init_global_executor`]. The lazy fallback that
/// previously lived here was removed: it called `std::process::exit(1)` from
/// library code, which is unacceptable. Setup failures now propagate as
/// errors through [`init_global_executor`] / [`global_executor`].
static GLOBAL_EXECUTOR: OnceLock<Mutex<ParallelExecutor>> = OnceLock::new();

/// Initializes the global parallel executor with the given configuration.
///
/// Must be called once during startup before [`global_executor`] is used.
/// Subsequent calls return an error. Setup failures (invalid config, thread
/// pool creation) are propagated instead of aborting the process.
///
/// # Arguments
/// * `config` - Configuration for the global executor. `None` uses defaults.
///
/// # Errors
/// Returns an error if:
/// - Global executor is already initialized
/// - Configuration validation fails
/// - Thread pool creation fails
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

/// Gets a reference to the global parallel executor, initializing it with
/// default configuration on first use.
///
/// Uses `OnceLock::get_or_try_init` so concurrent first-use callers (e.g.
/// parallel test threads) cannot race into the "already initialized" branch
/// the way a check-then-set sequence would. Any failure (e.g. thread pool
/// creation) is surfaced as an error rather than aborting the host process.
///
/// # Errors
/// Returns an error if the default-config executor cannot be constructed.
pub fn global_executor() -> Result<&'static Mutex<ParallelExecutor>> {
    GLOBAL_EXECUTOR.get_or_try_init(|| {
        let executor = ParallelExecutor::new(None)?;
        Ok::<_, anyhow::Error>(Mutex::new(executor))
    })
}
