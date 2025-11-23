//! Example demonstrating rate limiting features.
//!
//! This example shows how to:
//! - Use default rate limiting (enabled by default)
//! - Configure custom rate limit behavior
//! - Disable rate limiting
//! - Access rate limit information from errors
//! - Handle rate limit delays in retry logic
//!
//! Run with: `cargo run --example rate_limiting`

use calleen::{rate_limit::RateLimitConfig, Client, Error, RetryStrategy};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("calleen=debug")
        .init();

    println!("=== Example 1: Default Rate Limiting (Enabled) ===");
    println!("Rate limiting is enabled by default and will automatically");
    println!("respect Retry-After and X-RateLimit-* headers.\n");

    let client = Client::builder()
        .base_url("https://api.github.com")?
        .timeout(Duration::from_secs(10))
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_retries: 3,
            jitter: true,
        })
        .build()?;

    // This request might hit rate limits on GitHub's API
    match client.get::<serde_json::Value>("/rate_limit").await {
        Ok(response) => {
            println!("Success! Response status: {}", response.status);
            if let Some(remaining) = response.data.get("rate").and_then(|r| r.get("remaining")) {
                println!("Remaining API calls: {}", remaining);
            }
        }
        Err(Error::HttpError {
            status,
            rate_limit_info,
            ..
        }) if status.as_u16() == 429 => {
            println!("Rate limited! Status: {}", status);
            if let Some(info) = rate_limit_info {
                println!("Rate limit info:");
                if let Some(retry_after) = info.retry_after {
                    println!("  Retry after: {:?}", retry_after);
                }
                if let Some(reset_at) = info.reset_at {
                    println!("  Reset at: {:?}", reset_at);
                }
                if let Some(remaining) = info.remaining {
                    println!("  Remaining: {}", remaining);
                }
            }
        }
        Err(e) => println!("Error: {}", e),
    }
    println!();

    println!("=== Example 2: Custom Rate Limit Configuration ===");
    println!("You can customize rate limiting behavior, including maximum wait time.\n");

    let _client = Client::builder()
        .base_url("https://api.github.com")?
        .rate_limit_config(
            RateLimitConfig::builder()
                .enabled(true)
                .max_wait(Duration::from_secs(2)) // Cap rate limit waits at 2 seconds
                .respect_retry_after(true)
                .build(),
        )
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_retries: 2,
            jitter: false,
        })
        .build()?;

    println!("Client configured with:");
    println!("  - Rate limiting enabled");
    println!("  - Max wait capped at 2 seconds");
    println!("  - Respects Retry-After headers");
    println!();

    println!("=== Example 3: Disabling Rate Limiting ===");
    println!("You can disable automatic rate limit handling if needed.\n");

    let _client = Client::builder()
        .base_url("https://api.github.com")?
        .rate_limit_config(RateLimitConfig::builder().enabled(false).build())
        .build()?;

    println!("Client configured with rate limiting DISABLED");
    println!("Rate limit responses will be treated as regular HTTP errors.");
    println!();

    println!("=== Example 4: Accessing Rate Limit Info ===");
    println!("You can extract rate limit information from errors for custom handling.\n");

    let client = Client::builder()
        .base_url("https://httpbin.org")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
            max_retries: 1,
            jitter: false,
        })
        .build()?;

    // Simulate checking for rate limit info
    match client.get::<serde_json::Value>("/status/429").await {
        Ok(_) => println!("Unexpected success"),
        Err(e) => {
            println!("Error: {}", e);

            // Check if error has rate limit information
            if let Some(info) = e.rate_limit_info() {
                println!("\nRate limit details:");
                if let Some(retry_after) = info.retry_after {
                    println!("  Should retry after: {:?}", retry_after);
                }
                if let Some(reset_at) = info.reset_at {
                    if let Ok(duration_until_reset) = reset_at.duration_since(std::time::SystemTime::now()) {
                        println!("  Reset in: {:?}", duration_until_reset);
                    }
                }
                if let Some(remaining) = info.remaining {
                    println!("  Requests remaining: {}", remaining);
                }

                // Get recommended delay with custom max wait
                if let Some(delay) = e.rate_limit_delay(Duration::from_secs(10)) {
                    println!("  Recommended delay: {:?}", delay);
                }
            } else {
                println!("No rate limit information available");
            }

            // Check status code
            if let Some(status) = e.status() {
                println!("  Status code: {}", status);
            }
        }
    }
    println!();

    println!("=== Example 5: Rate Limiting with Retry Strategy ===");
    println!("Rate limit delays take precedence over normal retry delays.\n");

    let _client = Client::builder()
        .base_url("https://httpbin.org")?
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: 3,
            jitter: true,
        })
        .rate_limit_config(
            RateLimitConfig::builder()
                .enabled(true)
                .max_wait(Duration::from_secs(5))
                .build(),
        )
        .build()?;

    println!("When a rate limit is encountered:");
    println!("  1. Rate limit delay from Retry-After is preferred");
    println!("  2. Delay is capped by max_wait (5 seconds in this example)");
    println!("  3. If no rate limit info, normal retry strategy applies");
    println!("  4. Jitter is added to prevent thundering herd");

    Ok(())
}
