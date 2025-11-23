//! Example demonstrating custom retry predicates.
//!
//! This example shows how to:
//! - Create custom retry predicates
//! - Combine predicates with AND/OR logic
//! - Implement domain-specific retry logic
//! - Control retry behavior based on response details
//!
//! Run with: `cargo run --example custom_retry`

use calleen::retry::{
    AndPredicate, OrPredicate, RetryOn5xx, RetryOnTimeout, RetryPredicate,
};
use calleen::{Client, Error, RetryStrategy};
use std::time::Duration;

/// Custom predicate: Retry on rate limit errors (HTTP 429)
struct RetryOnRateLimit;

impl RetryPredicate for RetryOnRateLimit {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        matches!(
            error,
            Error::HttpError { status, .. } if status.as_u16() == 429
        )
    }
}

/// Custom predicate: Only retry for the first N attempts
struct MaxAttempts(usize);

impl RetryPredicate for MaxAttempts {
    fn should_retry(&self, _error: &Error, attempt: usize) -> bool {
        attempt <= self.0
    }
}

/// Custom predicate: Retry on specific error messages in the response
struct RetryOnErrorMessage {
    patterns: Vec<String>,
}

impl RetryPredicate for RetryOnErrorMessage {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        if let Some(raw_response) = error.raw_response() {
            self.patterns.iter().any(|pattern| raw_response.contains(pattern))
        } else {
            false
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter("calleen=info,custom_retry=info")
        .init();

    println!("=== Example 1: Retry on Rate Limits ===");
    let client_rate_limit = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            max_retries: 3,
            jitter: true,
        })
        .retry_predicate(Box::new(RetryOnRateLimit))
        .build()?;

    println!("This client will only retry on HTTP 429 (rate limit) errors");
    match client_rate_limit.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => println!("Success! Attempts: {}", response.attempts),
        Err(e) => println!("Failed: {}", e),
    }
    println!();

    println!("=== Example 2: Combining Predicates with OR ===");
    // Retry on either 5xx errors OR timeouts
    let or_predicate = OrPredicate::new(vec![
        Box::new(RetryOn5xx),
        Box::new(RetryOnTimeout),
        Box::new(RetryOnRateLimit),
    ]);

    let client_or = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(500),
            max_retries: 3,
        })
        .retry_predicate(Box::new(or_predicate))
        .build()?;

    println!("This client retries on: 5xx errors OR timeouts OR rate limits");
    match client_or.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => println!("Success! Attempts: {}", response.attempts),
        Err(e) => println!("Failed: {}", e),
    }
    println!();

    println!("=== Example 3: Combining Predicates with AND ===");
    // Retry on 5xx errors AND only for first 2 attempts
    let and_predicate = AndPredicate::new(vec![
        Box::new(RetryOn5xx),
        Box::new(MaxAttempts(2)),
    ]);

    let client_and = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(500),
            max_retries: 5, // Strategy allows 5, but predicate limits to 2
        })
        .retry_predicate(Box::new(and_predicate))
        .build()?;

    println!("This client retries on 5xx errors, but only for first 2 attempts");
    match client_and.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => println!("Success! Attempts: {}", response.attempts),
        Err(e) => println!("Failed: {}", e),
    }
    println!();

    println!("=== Example 4: Retry on Specific Error Messages ===");
    let message_predicate = RetryOnErrorMessage {
        patterns: vec![
            "timeout".to_string(),
            "temporarily unavailable".to_string(),
            "try again later".to_string(),
        ],
    };

    let client_message = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_retries: 3,
            jitter: true,
        })
        .retry_predicate(Box::new(message_predicate))
        .build()?;

    println!("This client retries when error messages contain specific patterns");
    match client_message.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => println!("Success! Attempts: {}", response.attempts),
        Err(e) => println!("Failed: {}", e),
    }
    println!();

    println!("=== Example 5: Complex Predicate Logic ===");
    // Retry if: (5xx OR timeout) AND attempt <= 3
    let complex_predicate = AndPredicate::new(vec![
        Box::new(OrPredicate::new(vec![
            Box::new(RetryOn5xx),
            Box::new(RetryOnTimeout),
        ])),
        Box::new(MaxAttempts(3)),
    ]);

    let client_complex = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            max_retries: 5,
            jitter: true,
        })
        .retry_predicate(Box::new(complex_predicate))
        .build()?;

    println!("This client uses complex retry logic: (5xx OR timeout) AND max 3 attempts");
    match client_complex.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => {
            println!("Success!");
            println!("  Attempts: {}", response.attempts);
            println!("  Latency: {:?}", response.latency);
        }
        Err(e) => println!("Failed: {}", e),
    }

    Ok(())
}
