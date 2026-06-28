use crate::parallel_executor::{BatchingMetrics, ExecutorConfig};
use std::sync::{Arc, Mutex};

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

        // Update stats if the lock is healthy. On poison we deliberately skip
        // ONLY the metrics update — real execution results must still flow
        // through. The previous code returned `Vec::new()` on poison, which
        // masqueraded as a successful empty batch and lost all real results.
        if let Ok(mut stats) = self.stats.lock() {
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
        }

        // Process batches in parallel (always, regardless of stats lock state)
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
