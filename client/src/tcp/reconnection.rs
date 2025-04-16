use rand;
use std::time::Duration;
use tracing::info;

use super::{tcp_client, tcp_client_blocking};

/// Configuration settings for connection retry and backoff strategy
#[derive(Debug, Clone)]
pub struct ReconnectionConfig {
    /// Base duration for reconnect interval (used with exponential backoff)
    pub base_interval: Duration,
    /// Maximum reconnect interval (cap for exponential backoff)
    pub max_interval: Duration,
    /// Maximum number of reconnection attempts.
    /// Use `None` for unlimited attempts.
    pub max_attempts: Option<u32>,
    /// Whether to add jitter to reconnection times (prevents thundering herd problem)
    pub use_jitter: bool,
}

impl Default for ReconnectionConfig {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            max_attempts: None,
            use_jitter: true,
        }
    }
}

/// Helper for managing reconnection attempts and backoff strategy
#[derive(Debug, Clone)]
pub struct ReconnectionManager {
    config: ReconnectionConfig,
    current_attempt: u32,
}

impl ReconnectionManager {
    /// Create a new reconnection manager with the given configuration
    pub fn new(config: ReconnectionConfig) -> Self {
        Self {
            config,
            current_attempt: 0,
        }
    }

    /// Create a new reconnection manager with default configuration
    pub fn new_default() -> Self {
        Self::new(ReconnectionConfig::default())
    }

    /// Get the current attempt number (zero-based)
    pub fn current_attempt(&self) -> u32 {
        self.current_attempt
    }

    /// Reset the attempt counter, typically called after a successful connection
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }

    /// Check if we've reached the maximum number of attempts
    ///
    /// Returns true if we've reached the maximum attempts, false if we can try again
    pub fn is_max_attempts_reached(&self) -> bool {
        if let Some(max) = self.config.max_attempts {
            self.current_attempt > max
        } else {
            false
        }
    }

    /// Increment the attempt counter and calculate the next retry delay with exponential backoff
    ///
    /// Returns None if max attempts reached, otherwise returns the Duration to wait
    pub fn next_delay(&mut self) -> Option<Duration> {
        // Increment the attempt counter first
        self.current_attempt += 1;

        // Check if we've now reached or exceeded the maximum number of attempts
        if let Some(max) = self.config.max_attempts {
            if self.current_attempt > max {
                return None;
            }
        }

        // Calculate exponential backoff with clamping to max_interval
        let exponent = std::cmp::min(self.current_attempt, 10); // Prevent potential overflow
        let backoff_secs = std::cmp::min(
            self.config.base_interval.as_secs() * (1 << exponent),
            self.config.max_interval.as_secs(),
        );

        // Add jitter if configured (to prevent thundering herd problem)
        let final_secs = if self.config.use_jitter {
            let jitter = rand::random::<u64>() % (backoff_secs / 4 + 1);
            backoff_secs + jitter
        } else {
            backoff_secs
        };

        let wait_time = Duration::from_secs(final_secs);

        info!(
            "Reconnection attempt {}/{:?} scheduled in {:?}",
            self.current_attempt, self.config.max_attempts, wait_time
        );

        Some(wait_time)
    }

    /// Get the configuration
    pub fn config(&self) -> &ReconnectionConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: ReconnectionConfig) {
        self.config = config;
    }
}

/// Convert from the sync client's TcpClientConfig to ReconnectionConfig
pub fn from_tcp_client_config(config: &tcp_client_blocking::TcpClientConfig) -> ReconnectionConfig {
    ReconnectionConfig {
        base_interval: config.base_reconnect_interval,
        max_interval: config.max_reconnect_interval,
        max_attempts: config.max_reconnect_attempts,
        use_jitter: true,
    }
}

/// Convert from the async client's ReconnectSettings to ReconnectionConfig
pub fn from_async_reconnect_settings(
    settings: &tcp_client::Config,
) -> ReconnectionConfig {
    ReconnectionConfig {
        base_interval: settings.retry_delay,
        max_interval: settings.retry_delay, // No max interval in async client, use same as base
        max_attempts: settings.max_retries,
        use_jitter: false, // Async client doesn't use jitter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let config = ReconnectionConfig {
            base_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            max_attempts: Some(10),
            use_jitter: false, // Disable jitter for predictable test results
        };

        let mut manager = ReconnectionManager::new(config);

        // First attempt should be 1s * 2^1 = 2s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(2));

        // Second attempt should be 1s * 2^2 = 4s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(4));

        // Third attempt should be 1s * 2^3 = 8s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(8));

        // Reset should set attempt back to 0
        manager.reset();
        assert_eq!(manager.current_attempt, 0);

        // After reset, next attempt should be 1s * 2^1 = 2s again
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(2));
    }

    #[test]
    fn test_max_attempts() {
        let config = ReconnectionConfig {
            base_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            max_attempts: Some(3),
            use_jitter: false,
        };

        let mut manager = ReconnectionManager::new(config);

        // We should get delays for 3 attempts
        assert!(manager.next_delay().is_some()); // Attempt 1
        assert!(manager.next_delay().is_some()); // Attempt 2
        assert!(manager.next_delay().is_some()); // Attempt 3

        // Then we should get None
        assert!(manager.next_delay().is_none());
    }

    #[test]
    fn test_unlimited_attempts() {
        let config = ReconnectionConfig {
            base_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            max_attempts: None,
            use_jitter: false,
        };

        let mut manager = ReconnectionManager::new(config);

        // We should be able to get many delays without hitting a limit
        for _ in 0..100 {
            assert!(manager.next_delay().is_some());
        }
    }

    #[test]
    fn test_max_interval() {
        let config = ReconnectionConfig {
            base_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(8),
            max_attempts: None,
            use_jitter: false,
        };

        let mut manager = ReconnectionManager::new(config);

        // 1st attempt: 1s * 2^1 = 2s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(2));

        // 2nd attempt: 1s * 2^2 = 4s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(4));

        // 3rd attempt: 1s * 2^3 = 8s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(8));

        // 4th attempt: should be capped at 8s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(8));

        // 5th attempt: should still be capped at 8s
        assert_eq!(manager.next_delay().unwrap(), Duration::from_secs(8));
    }
}
