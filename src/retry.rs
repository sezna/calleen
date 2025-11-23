//! Retry strategies and predicates for handling transient failures.
//!
//! This module provides flexible retry logic with various strategies and
//! customizable predicates for determining when to retry failed requests.

use crate::Error;
use rand::Rng;
use std::time::Duration;

/// Defines when and how to retry failed requests.
///
/// # Examples
///
/// ```
/// use calleen::RetryStrategy;
/// use std::time::Duration;
///
/// // No retries
/// let no_retry = RetryStrategy::None;
///
/// // Exponential backoff: 100ms, 200ms, 400ms, 800ms...
/// let exponential = RetryStrategy::ExponentialBackoff {
///     initial_delay: Duration::from_millis(100),
///     max_delay: Duration::from_secs(30),
///     max_retries: 5,
///     jitter: true,
/// };
///
/// // Linear backoff: 1s, 1s, 1s...
/// let linear = RetryStrategy::Linear {
///     delay: Duration::from_secs(1),
///     max_retries: 3,
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub enum RetryStrategy {
    /// Do not retry failed requests.
    #[default]
    None,

    /// Retry with exponentially increasing delays.
    ///
    /// Each retry waits for `initial_delay * 2^attempt` (capped at `max_delay`).
    /// Optional jitter adds randomness to prevent thundering herd.
    ExponentialBackoff {
        /// The initial delay before the first retry.
        initial_delay: Duration,
        /// The maximum delay between retries.
        max_delay: Duration,
        /// The maximum number of retry attempts.
        max_retries: usize,
        /// Whether to add random jitter to delays (recommended).
        jitter: bool,
    },

    /// Retry with a fixed delay between attempts.
    Linear {
        /// The delay between retry attempts.
        delay: Duration,
        /// The maximum number of retry attempts.
        max_retries: usize,
    },

    /// Custom retry logic.
    ///
    /// Provide a function that takes the attempt number (starting from 1)
    /// and returns `Some(delay)` to retry after the delay, or `None` to stop.
    Custom {
        /// Function that determines retry delay.
        ///
        /// Takes the attempt number (1-indexed) and returns the delay
        /// before that attempt, or `None` to stop retrying.
        delay_fn: fn(attempt: usize) -> Option<Duration>,
    },
}

impl RetryStrategy {
    /// Returns the delay before the given retry attempt, or `None` if retries are exhausted.
    ///
    /// # Arguments
    ///
    /// * `attempt` - The retry attempt number (1-indexed, so 1 = first retry)
    pub fn delay_for_attempt(&self, attempt: usize) -> Option<Duration> {
        match self {
            RetryStrategy::None => None,
            RetryStrategy::ExponentialBackoff {
                initial_delay,
                max_delay,
                max_retries,
                jitter,
            } => {
                if attempt > *max_retries {
                    return None;
                }

                // Calculate base delay: initial_delay * 2^(attempt - 1)
                let multiplier = 2u64.saturating_pow(attempt.saturating_sub(1) as u32);
                let base_delay =
                    initial_delay.saturating_mul(multiplier.try_into().unwrap_or(u32::MAX));
                let delay = base_delay.min(*max_delay);

                if *jitter {
                    // Add jitter: random value between 50% and 100% of the delay
                    let jitter_factor = rand::thread_rng().gen_range(0.5..=1.0);
                    Some(delay.mul_f64(jitter_factor))
                } else {
                    Some(delay)
                }
            }
            RetryStrategy::Linear { delay, max_retries } => {
                if attempt > *max_retries {
                    None
                } else {
                    Some(*delay)
                }
            }
            RetryStrategy::Custom { delay_fn } => delay_fn(attempt),
        }
    }

    /// Returns the maximum number of retries, if applicable.
    pub fn max_retries(&self) -> Option<usize> {
        match self {
            RetryStrategy::None => Some(0),
            RetryStrategy::ExponentialBackoff { max_retries, .. } => Some(*max_retries),
            RetryStrategy::Linear { max_retries, .. } => Some(*max_retries),
            RetryStrategy::Custom { .. } => None,
        }
    }
}

/// Trait for determining whether a failed request should be retried.
///
/// Implement this trait to create custom retry logic based on the error type,
/// response status, headers, or any other criteria.
///
/// # Examples
///
/// ```
/// use calleen::{Error, RetryPredicate};
///
/// struct RetryOnRateLimit;
///
/// impl RetryPredicate for RetryOnRateLimit {
///     fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
///         matches!(
///             error,
///             Error::HttpError { status, .. } if status.as_u16() == 429
///         )
///     }
/// }
/// ```
pub trait RetryPredicate: Send + Sync {
    /// Determines whether the request should be retried based on the error.
    ///
    /// # Arguments
    ///
    /// * `error` - The error that occurred
    /// * `attempt` - The attempt number (1-indexed)
    ///
    /// # Returns
    ///
    /// `true` if the request should be retried, `false` otherwise.
    fn should_retry(&self, error: &Error, attempt: usize) -> bool;
}

/// Retry all errors that are marked as retryable.
///
/// This uses the `Error::is_retryable()` method, which returns `true` for
/// network errors, timeouts, and 5xx HTTP errors.
#[derive(Debug, Clone, Copy)]
pub struct RetryOnRetryable;

impl RetryPredicate for RetryOnRetryable {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        error.is_retryable()
    }
}

/// Retry only on 5xx server errors.
#[derive(Debug, Clone, Copy)]
pub struct RetryOn5xx;

impl RetryPredicate for RetryOn5xx {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        matches!(error, Error::HttpError { status, .. } if status.is_server_error())
    }
}

/// Retry only on timeout errors.
#[derive(Debug, Clone, Copy)]
pub struct RetryOnTimeout;

impl RetryPredicate for RetryOnTimeout {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        matches!(error, Error::Timeout)
    }
}

/// Retry only on network/connection errors.
#[derive(Debug, Clone, Copy)]
pub struct RetryOnConnectionError;

impl RetryPredicate for RetryOnConnectionError {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        matches!(error, Error::Network(_))
    }
}

/// Combine multiple retry predicates with OR logic.
///
/// Retries if ANY of the predicates return `true`.
///
/// # Examples
///
/// ```
/// use calleen::retry::{RetryOn5xx, RetryOnTimeout, OrPredicate};
///
/// // Retry on 5xx errors OR timeouts
/// let predicate = OrPredicate::new(vec![
///     Box::new(RetryOn5xx),
///     Box::new(RetryOnTimeout),
/// ]);
/// ```
pub struct OrPredicate {
    predicates: Vec<Box<dyn RetryPredicate>>,
}

impl OrPredicate {
    /// Creates a new `OrPredicate` from a list of predicates.
    pub fn new(predicates: Vec<Box<dyn RetryPredicate>>) -> Self {
        Self { predicates }
    }
}

impl RetryPredicate for OrPredicate {
    fn should_retry(&self, error: &Error, attempt: usize) -> bool {
        self.predicates
            .iter()
            .any(|p| p.should_retry(error, attempt))
    }
}

/// Combine multiple retry predicates with AND logic.
///
/// Retries only if ALL of the predicates return `true`.
///
/// # Examples
///
/// ```
/// use calleen::retry::{RetryOn5xx, AndPredicate};
/// use calleen::{Error, RetryPredicate};
///
/// struct MaxAttempts(usize);
///
/// impl RetryPredicate for MaxAttempts {
///     fn should_retry(&self, _error: &Error, attempt: usize) -> bool {
///         attempt <= self.0
///     }
/// }
///
/// // Retry on 5xx errors AND only for first 3 attempts
/// let predicate = AndPredicate::new(vec![
///     Box::new(RetryOn5xx),
///     Box::new(MaxAttempts(3)),
/// ]);
/// ```
pub struct AndPredicate {
    predicates: Vec<Box<dyn RetryPredicate>>,
}

impl AndPredicate {
    /// Creates a new `AndPredicate` from a list of predicates.
    pub fn new(predicates: Vec<Box<dyn RetryPredicate>>) -> Self {
        Self { predicates }
    }
}

impl RetryPredicate for AndPredicate {
    fn should_retry(&self, error: &Error, attempt: usize) -> bool {
        self.predicates
            .iter()
            .all(|p| p.should_retry(error, attempt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff_delays() {
        let strategy = RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            max_retries: 5,
            jitter: false,
        };

        assert_eq!(
            strategy.delay_for_attempt(1),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            strategy.delay_for_attempt(2),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            strategy.delay_for_attempt(3),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            strategy.delay_for_attempt(4),
            Some(Duration::from_millis(800))
        );
        assert_eq!(
            strategy.delay_for_attempt(5),
            Some(Duration::from_millis(1600))
        );
        assert_eq!(strategy.delay_for_attempt(6), None);
    }

    #[test]
    fn test_linear_delays() {
        let strategy = RetryStrategy::Linear {
            delay: Duration::from_secs(1),
            max_retries: 3,
        };

        assert_eq!(strategy.delay_for_attempt(1), Some(Duration::from_secs(1)));
        assert_eq!(strategy.delay_for_attempt(2), Some(Duration::from_secs(1)));
        assert_eq!(strategy.delay_for_attempt(3), Some(Duration::from_secs(1)));
        assert_eq!(strategy.delay_for_attempt(4), None);
    }

    #[test]
    fn test_no_retry() {
        let strategy = RetryStrategy::None;
        assert_eq!(strategy.delay_for_attempt(1), None);
    }
}
