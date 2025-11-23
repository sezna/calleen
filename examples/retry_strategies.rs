//! Example demonstrating different retry strategies.
//!
//! This example shows how to:
//! - Configure exponential backoff retries
//! - Configure linear retries
//! - Configure custom retry logic
//! - Control retry behavior with predicates
//!
//! Run with: `cargo run --example retry_strategies`

use calleen::{Client, Error, RetryStrategy};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing to see retry attempts
    tracing_subscriber::fmt()
        .with_env_filter("calleen=info,retry_strategies=info")
        .init();

    println!("=== No Retry Strategy ===");
    let client_no_retry = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::None)
        .build()?;

    // This will fail immediately if the endpoint doesn't exist
    match client_no_retry
        .get::<serde_json::Value>("/nonexistent")
        .await
    {
        Ok(_) => println!("Unexpected success"),
        Err(e) => println!("Failed immediately (no retries): {}", e),
    }
    println!();

    println!("=== Exponential Backoff Strategy ===");
    println!("Delays: 100ms, 200ms, 400ms (with jitter)");
    let client_exponential = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: 3,
            jitter: true, // Adds randomness to prevent thundering herd
        })
        .timeout(Duration::from_secs(5))
        .build()?;

    // Try a failing request to see retry behavior
    // Note: This will retry but still fail after exhausting retries
    let start = std::time::Instant::now();
    match client_exponential
        .get::<serde_json::Value>("/posts/999999")
        .await
    {
        Ok(response) => println!("Response: {:?}", response.data),
        Err(e) => {
            println!("Failed after retries: {}", e);
            println!("Total time: {:?}", start.elapsed());
        }
    }
    println!();

    println!("=== Linear Retry Strategy ===");
    println!("Fixed 500ms delay between attempts");
    let client_linear = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(500),
            max_retries: 3,
        })
        .timeout(Duration::from_secs(5))
        .build()?;

    let start = std::time::Instant::now();
    match client_linear.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => {
            println!("Success!");
            println!("Attempts: {}", response.attempts);
            println!("Total time: {:?}", start.elapsed());
        }
        Err(e) => println!("Failed: {}", e),
    }
    println!();

    println!("=== Custom Retry Strategy ===");
    println!("Custom delay function: attempt 1=100ms, 2=300ms, 3=1000ms");
    let client_custom = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .retry_strategy(RetryStrategy::Custom {
            delay_fn: |attempt| match attempt {
                1 => Some(Duration::from_millis(100)),
                2 => Some(Duration::from_millis(300)),
                3 => Some(Duration::from_millis(1000)),
                _ => None, // Stop retrying after 3 attempts
            },
        })
        .build()?;

    match client_custom.get::<serde_json::Value>("/posts/1").await {
        Ok(response) => {
            println!("Success with custom strategy!");
            println!("Attempts: {}", response.attempts);
        }
        Err(e) => println!("Failed: {}", e),
    }

    Ok(())
}
