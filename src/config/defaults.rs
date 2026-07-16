//! Default implementations for the environmental config structs.

use std::net::SocketAddr;

use crate::config::*;

/// The default whole-request HTTP timeout, shared by the model and embedding clients. Long enough
/// for a local model's worst-case prefill-plus-generation; short enough that a hung backend becomes
/// a retryable timeout rather than a forever-stall.
pub(crate) const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 300;

impl Default for ResilienceConfig {
    fn default() -> Self {
        ResilienceConfig {
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            max_attempts: 3,
            backoff_base_ms: 500,
            backoff_max_ms: 10_000,
            breaker_failure_threshold: 3,
            breaker_open_seconds: 30,
        }
    }
}

impl Default for ServingConfig {
    fn default() -> Self {
        ServingConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 7777)),
            control_keys: Vec::new(),
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            endpoint: String::new(),
            model: String::new(),
            dimensions: 0,
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            context_length: None,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            dir: PathBuf::from("data"),
        }
    }
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        SnapshotConfig {
            enabled: true,
            dir: None,
            check_interval_seconds: 3_600,
            min_new_events: 20,
            keep: 5,
        }
    }
}
