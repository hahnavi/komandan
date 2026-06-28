use serde::{Deserialize, Serialize};

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
        let cpu_count = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        // 2x CPU count, floored at 4 — small CI runners report 1-2 cores and
        // the I/O-bound preset still benefits from at least 4 worker threads.
        let thread_count = std::cmp::max(cpu_count * 2, 4);

        Self {
            thread_count: Some(thread_count), // More threads for I/O waiting
            chunk_size: Some(50),             // Smaller chunks for better responsiveness
            timeout_seconds: Some(900),       // 15 minutes for network operations
            error_strategy: Some("continue".to_string()),
            max_memory_mb: Some(256), // Lower memory per thread
        }
    }

    /// Gets the effective thread count (resolves None to CPU core count)
    #[must_use]
    pub fn effective_thread_count(&self) -> usize {
        self.thread_count.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(4, std::num::NonZero::get) // Fallback to 4 threads if detection fails
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
