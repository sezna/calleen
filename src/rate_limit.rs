//! Rate limiting support with automatic header parsing.
//!
//! This module provides automatic rate limit handling by parsing common
//! rate limit headers from HTTP responses and respecting the indicated wait times.

use http::HeaderMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Information extracted from rate limit headers.
///
/// This struct contains parsed rate limit data from various standard and
/// common rate limit headers.
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// When the rate limit resets (from X-RateLimit-Reset or RateLimit-Reset headers).
    pub reset_at: Option<SystemTime>,

    /// How long to wait before retrying (from Retry-After header).
    pub retry_after: Option<Duration>,

    /// Number of requests remaining in the current window.
    pub remaining: Option<u64>,
}

impl RateLimitInfo {
    /// Extracts rate limit information from HTTP response headers.
    ///
    /// Parses common rate limit headers including:
    /// - `Retry-After` (standard HTTP, seconds or HTTP date)
    /// - `X-RateLimit-Reset` (Unix timestamp)
    /// - `RateLimit-Reset` (draft standard, Unix timestamp)
    /// - `X-RateLimit-Remaining`
    ///
    /// # Examples
    ///
    /// ```
    /// use calleen::rate_limit::RateLimitInfo;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert("retry-after", "60".parse().unwrap());
    /// headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
    ///
    /// let info = RateLimitInfo::from_headers(&headers);
    /// assert!(info.retry_after.is_some());
    /// ```
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let retry_after = parse_retry_after(headers);
        let reset_at = parse_rate_limit_reset(headers);
        let remaining = parse_rate_limit_remaining(headers);

        Self {
            reset_at,
            retry_after,
            remaining,
        }
    }

    /// Returns the recommended delay before retrying.
    ///
    /// This uses `retry_after` if available, otherwise calculates from `reset_at`.
    /// Returns `None` if no rate limit information is available.
    ///
    /// The delay is capped by the provided `max_wait` duration.
    pub fn delay(&self, max_wait: Duration) -> Option<Duration> {
        // Prefer explicit Retry-After header
        if let Some(retry_after) = self.retry_after {
            return Some(retry_after.min(max_wait));
        }

        // Fall back to calculating from reset time
        if let Some(reset_at) = self.reset_at {
            if let Ok(until_reset) = reset_at.duration_since(SystemTime::now()) {
                return Some(until_reset.min(max_wait));
            }
        }

        None
    }

    /// Returns `true` if this represents an active rate limit.
    ///
    /// A rate limit is considered active if:
    /// - `retry_after` is specified, OR
    /// - `remaining` is Some(0)
    pub fn is_rate_limited(&self) -> bool {
        self.retry_after.is_some() || self.remaining == Some(0)
    }
}

/// Configuration for rate limit handling.
///
/// # Examples
///
/// ```
/// use calleen::rate_limit::RateLimitConfig;
/// use std::time::Duration;
///
/// let config = RateLimitConfig::builder()
///     .enabled(true)
///     .max_wait(Duration::from_secs(300))
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Whether to automatically handle rate limits.
    ///
    /// When enabled, the client will parse rate limit headers and wait
    /// the indicated time before retrying.
    pub enabled: bool,

    /// Maximum time to wait for a rate limit reset.
    ///
    /// This prevents waiting indefinitely for rate limits. Defaults to 5 minutes.
    pub max_wait: Duration,

    /// Whether to respect the Retry-After header.
    ///
    /// Defaults to `true`.
    pub respect_retry_after: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_wait: Duration::from_secs(300), // 5 minutes
            respect_retry_after: true,
        }
    }
}

impl RateLimitConfig {
    /// Creates a new builder for configuring rate limit handling.
    pub fn builder() -> RateLimitConfigBuilder {
        RateLimitConfigBuilder::default()
    }

    /// Creates a disabled rate limit configuration.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Builder for `RateLimitConfig`.
#[derive(Default)]
pub struct RateLimitConfigBuilder {
    enabled: Option<bool>,
    max_wait: Option<Duration>,
    respect_retry_after: Option<bool>,
}

impl RateLimitConfigBuilder {
    /// Sets whether rate limit handling is enabled.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    /// Sets the maximum wait time for rate limits.
    pub fn max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = Some(max_wait);
        self
    }

    /// Sets whether to respect the Retry-After header.
    pub fn respect_retry_after(mut self, respect: bool) -> Self {
        self.respect_retry_after = Some(respect);
        self
    }

    /// Builds the `RateLimitConfig`.
    pub fn build(self) -> RateLimitConfig {
        let default = RateLimitConfig::default();
        RateLimitConfig {
            enabled: self.enabled.unwrap_or(default.enabled),
            max_wait: self.max_wait.unwrap_or(default.max_wait),
            respect_retry_after: self
                .respect_retry_after
                .unwrap_or(default.respect_retry_after),
        }
    }
}

/// Parses the Retry-After header.
///
/// Supports both delay-seconds (integer) and HTTP-date formats.
fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let header = headers.get("retry-after")?.to_str().ok()?;

    // Try parsing as seconds (integer)
    if let Ok(seconds) = header.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    // Try parsing as HTTP date (RFC 7231 format)
    if let Ok(date_time) = httpdate::parse_http_date(header) {
        if let Ok(duration) = date_time.duration_since(SystemTime::now()) {
            return Some(duration);
        }
    }

    None
}

/// Parses X-RateLimit-Reset or RateLimit-Reset headers (Unix timestamp).
fn parse_rate_limit_reset(headers: &HeaderMap) -> Option<SystemTime> {
    // Try X-RateLimit-Reset first (more common)
    if let Some(header) = headers.get("x-ratelimit-reset") {
        if let Ok(timestamp_str) = header.to_str() {
            if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                return Some(UNIX_EPOCH + Duration::from_secs(timestamp));
            }
        }
    }

    // Try RateLimit-Reset (draft standard)
    if let Some(header) = headers.get("ratelimit-reset") {
        if let Ok(timestamp_str) = header.to_str() {
            if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                return Some(UNIX_EPOCH + Duration::from_secs(timestamp));
            }
        }
    }

    None
}

/// Parses X-RateLimit-Remaining header.
fn parse_rate_limit_remaining(headers: &HeaderMap) -> Option<u64> {
    let header = headers.get("x-ratelimit-remaining")?.to_str().ok()?;
    header.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn test_parse_retry_after_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("60"));

        let delay = parse_retry_after(&headers);
        assert_eq!(delay, Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_rate_limit_info_with_reset_and_remaining() {
        let mut headers = HeaderMap::new();
        let now = SystemTime::now();
        // Use 2 seconds to give more time tolerance
        let future_time = now + Duration::from_secs(2);
        let future_timestamp = future_time.duration_since(UNIX_EPOCH).unwrap().as_secs();

        headers.insert(
            "x-ratelimit-reset",
            HeaderValue::from_str(&future_timestamp.to_string()).unwrap(),
        );
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));

        let info = RateLimitInfo::from_headers(&headers);
        assert!(info.reset_at.is_some());
        assert_eq!(info.remaining, Some(0));
        assert!(
            info.is_rate_limited(),
            "Should be rate limited when remaining=0"
        );

        let delay = info.delay(Duration::from_secs(300));
        assert!(delay.is_some(), "Should have a delay");
        if let Some(d) = delay {
            // Should be close to 2 seconds
            // Note: Unix timestamps are in whole seconds, so nanoseconds are truncated,
            // which can reduce the delay by up to 1 second
            assert!(
                d >= Duration::from_secs(1) && d <= Duration::from_secs(3),
                "Delay should be 1-3 seconds, got {:?}",
                d
            );
        }
    }

    #[test]
    fn test_parse_rate_limit_reset() {
        let mut headers = HeaderMap::new();
        let future_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 120;
        headers.insert(
            "x-ratelimit-reset",
            HeaderValue::from_str(&future_timestamp.to_string()).unwrap(),
        );

        let reset_at = parse_rate_limit_reset(&headers);
        assert!(reset_at.is_some());
    }

    #[test]
    fn test_parse_rate_limit_remaining() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("42"));

        let remaining = parse_rate_limit_remaining(&headers);
        assert_eq!(remaining, Some(42));
    }

    #[test]
    fn test_rate_limit_info_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("30"));
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));

        let info = RateLimitInfo::from_headers(&headers);
        assert!(info.retry_after.is_some());
        assert_eq!(info.remaining, Some(0));
        assert!(info.is_rate_limited());
    }

    #[test]
    fn test_rate_limit_delay_capped_by_max_wait() {
        let info = RateLimitInfo {
            reset_at: None,
            retry_after: Some(Duration::from_secs(600)),
            remaining: Some(0),
        };

        let delay = info.delay(Duration::from_secs(300));
        assert_eq!(delay, Some(Duration::from_secs(300)));
    }
}
