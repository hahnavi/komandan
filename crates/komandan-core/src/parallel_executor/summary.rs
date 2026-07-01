use crate::parallel_executor::ExecutionResult;
use mlua::{Lua, Table};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

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
    pub(crate) fn categorize_error(error: &str) -> String {
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
