//! Example demonstrating comprehensive error handling.
//!
//! This example shows how to:
//! - Handle different error types
//! - Access raw response data on errors
//! - Inspect HTTP status codes and headers
//! - Deal with deserialization failures
//! - Check if errors are retryable
//!
//! Run with: `cargo run --example error_handling`

use calleen::{Client, Error};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Post {
    id: u32,
    title: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("calleen=info")
        .init();

    let client = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .build()?;

    println!("=== Example 1: Handling HTTP Errors ===");
    // Try to fetch a non-existent resource (404 error)
    match client.get::<Post>("/posts/999999").await {
        Ok(response) => println!("Success: {:?}", response.data),
        Err(Error::HttpError {
            status,
            raw_response,
            headers,
            ..
        }) => {
            println!("HTTP Error!");
            println!("  Status: {}", status);
            println!("  Status code: {}", status.as_u16());
            println!("  Is client error (4xx): {}", status.is_client_error());
            println!("  Is server error (5xx): {}", status.is_server_error());
            println!("  Raw response: {}", raw_response);
            println!("  Content-Type: {:?}", headers.get("content-type"));
        }
        Err(e) => println!("Other error: {}", e),
    }
    println!();

    println!("=== Example 2: Handling Deserialization Errors ===");
    // Define a struct that doesn't match the API response
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct WrongSchema {
        nonexistent_field: String,
    }

    match client.get::<WrongSchema>("/posts/1").await {
        Ok(_) => println!("Unexpected success"),
        Err(Error::DeserializationFailed {
            raw_response,
            serde_error,
            status,
        }) => {
            println!("Deserialization Failed!");
            println!("  Status: {}", status);
            println!("  Serde error: {}", serde_error);
            println!(
                "  Raw response (first 200 chars): {}",
                raw_response.chars().take(200).collect::<String>()
            );
            println!("\nThis is incredibly useful for debugging API schema mismatches!");
        }
        Err(e) => println!("Other error: {}", e),
    }
    println!();

    println!("=== Example 3: Checking Error Retryability ===");
    // Demonstrate error inspection
    let errors = vec![
        Error::HttpError {
            status: http::StatusCode::INTERNAL_SERVER_ERROR,
            raw_response: "Server error".to_string().into_boxed_str(),
            headers: Box::new(http::HeaderMap::new()),
            rate_limit_info: None,
        },
        Error::HttpError {
            status: http::StatusCode::BAD_REQUEST,
            raw_response: "Bad request".to_string().into_boxed_str(),
            headers: Box::new(http::HeaderMap::new()),
            rate_limit_info: None,
        },
        Error::Timeout,
        Error::ConfigurationError("Invalid config".to_string()),
    ];

    for error in errors {
        println!("Error: {}", error);
        println!("  Is retryable: {}", error.is_retryable());
        println!("  Status code: {:?}", error.status());
        println!("  Raw response: {:?}", error.raw_response());
        println!();
    }

    println!("=== Example 4: Handling Network Errors ===");
    // Try to connect to an invalid URL
    let bad_client = Client::builder()
        .base_url("https://this-domain-does-not-exist-12345.com")?
        .build()?;

    match bad_client.get::<serde_json::Value>("/").await {
        Ok(_) => println!("Unexpected success"),
        Err(Error::Network(e)) => {
            println!("Network Error!");
            println!("  Error: {}", e);
            println!("  Is timeout: {}", e.is_timeout());
            println!("  Is connect error: {}", e.is_connect());
        }
        Err(e) => println!("Other error: {}", e),
    }
    println!();

    println!("=== Example 5: Using Error Methods ===");
    match client.get::<Post>("/posts/999999").await {
        Ok(_) => {}
        Err(e) => {
            println!("Error occurred: {}", e);

            // Check if we can retry
            if e.is_retryable() {
                println!("  This error is retryable (5xx, timeout, or network issue)");
            } else {
                println!("  This error is NOT retryable (4xx or other permanent failure)");
            }

            // Get status if available
            if let Some(status) = e.status() {
                println!("  HTTP status: {}", status);
            }

            // Get raw response if available
            if let Some(raw) = e.raw_response() {
                println!("  Raw response available: {} bytes", raw.len());
            }
        }
    }

    Ok(())
}
