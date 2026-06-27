use crate::parallel_executor::{ExecutorConfig, ParallelExecutorError};
use anyhow::Result;

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
pub fn validate_config(config: &ExecutorConfig) -> Result<()> {
    // Validate thread count
    if let Some(thread_count) = config.thread_count {
        if thread_count == 0 {
            return Err(ParallelExecutorError::Configuration {
                message: "thread_count must be greater than 0".to_string(),
                parameter: Some("thread_count".to_string()),
            }
            .into());
        }

        if thread_count > 1024 {
            return Err(ParallelExecutorError::Configuration {
                message: format!("thread_count {thread_count} exceeds maximum limit of 1024"),
                parameter: Some("thread_count".to_string()),
            }
            .into());
        }

        // Warn about excessive thread counts relative to CPU cores
        let cpu_cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);

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
                parameter: Some("chunk_size".to_string()),
            }
            .into());
        }

        if chunk_size > 100_000 {
            return Err(ParallelExecutorError::Configuration {
                message: format!("chunk_size {chunk_size} is excessively large"),
                parameter: Some("chunk_size".to_string()),
            }
            .into());
        }
    }

    // Validate timeout
    if let Some(timeout_seconds) = config.timeout_seconds {
        if timeout_seconds == 0 {
            return Err(ParallelExecutorError::Configuration {
                message: "timeout_seconds must be greater than 0".to_string(),
                parameter: Some("timeout_seconds".to_string()),
            }
            .into());
        }

        if timeout_seconds > 86400 {
            return Err(ParallelExecutorError::Configuration {
                message: format!(
                    "timeout_seconds {timeout_seconds} exceeds maximum of 24 hours (86400 seconds)"
                ),
                parameter: Some("timeout_seconds".to_string()),
            }
            .into());
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
                    parameter: Some("error_strategy".to_string()),
                }
                .into());
            }
        }
    }

    // Validate memory limits
    if let Some(max_memory_mb) = config.max_memory_mb {
        if max_memory_mb == 0 {
            return Err(ParallelExecutorError::Configuration {
                message: "max_memory_mb must be greater than 0".to_string(),
                parameter: Some("max_memory_mb".to_string()),
            }
            .into());
        }

        if max_memory_mb > 16384 {
            return Err(ParallelExecutorError::Configuration {
                message: format!("max_memory_mb {max_memory_mb} exceeds reasonable limit of 16GB"),
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
    validate_config_combinations(config);

    Ok(())
}

/// Validates configuration parameter combinations for optimal performance
pub fn validate_config_combinations(config: &ExecutorConfig) {
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
