use crate::parallel_executor::{
    BatchingMetrics, ConnectionStats, MemoryUsage, PerformanceMetrics, ThroughputMetrics,
};
use std::time::{Duration, Instant};

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
    ///
    /// `sequential_work` is the sum of per-item execution times (the work that
    /// would have been done serially). It lets us derive a real `speedup_factor`
    /// instead of the previous algebraically-cancelled `thread_count` constant.
    pub fn finish_processing(
        &mut self,
        successful_count: usize,
        sequential_work: Duration,
    ) -> PerformanceMetrics {
        let total_time = self.start_time.elapsed();

        let memory_usage = self.memory_tracker.get_usage();
        let throughput =
            self.throughput_calculator
                .calculate(total_time, successful_count, sequential_work);

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
                / std::thread::available_parallelism().map_or(1, std::num::NonZero::get) as f64,
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

    fn calculate(
        &self,
        total_time: Duration,
        successful_count: usize,
        sequential_work: Duration,
    ) -> ThroughputMetrics {
        #[allow(clippy::cast_precision_loss)]
        let items_per_second = if total_time.as_secs_f64() > 0.0 {
            successful_count as f64 / total_time.as_secs_f64()
        } else {
            0.0
        };

        // Real speedup: how much faster we were vs running the same work
        // serially. Sequential work is the sum of per-item execution times
        // (already collected in `ExecutionResult::execution_time`); total_time
        // is the wall-clock parallel elapsed time. Both grounded in measured
        // data instead of the previous `total_time * thread_count / total_time`
        // expression that always collapsed to `thread_count`.
        let speedup_factor = if total_time.as_secs_f64() > 0.0 {
            sequential_work.as_secs_f64() / total_time.as_secs_f64()
        } else {
            1.0
        };

        // CPU efficiency: fraction of theoretical parallel speedup we achieved.
        #[allow(clippy::cast_precision_loss)]
        let theoretical_max_speedup = self.thread_count as f64;
        let cpu_efficiency = if theoretical_max_speedup > 0.0 {
            (speedup_factor / theoretical_max_speedup).clamp(0.0, 1.0)
        } else {
            0.0
        };

        ThroughputMetrics {
            items_per_second,
            speedup_factor,
            cpu_efficiency,
        }
    }
}
