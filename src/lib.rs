//! # Calleen - A robust HTTP API client library
//!
//! Calleen is a type-safe, retry-aware HTTP client library built on top of `reqwest`.
//! It provides rich error handling, automatic retries, request/response logging, and
//! preserves raw response data for debugging.
//!
//! ## Quick Start
//!
//! ```no_run
//! use calleen::{Client, RetryStrategy};
//! use serde::{Deserialize, Serialize};
//! use std::time::Duration;
//!
//! #[derive(Serialize)]
//! struct CreateUser {
//!     name: String,
//!     email: String,
//! }
//!
//! #[derive(Deserialize)]
//! struct User {
//!     id: u64,
//!     name: String,
//!     email: String,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), calleen::Error> {
//!     // Create a client with retry logic
//!     let client = Client::builder()
//!         .base_url("https://api.example.com")?
//!         .timeout(Duration::from_secs(30))
//!         .retry_strategy(RetryStrategy::ExponentialBackoff {
//!             initial_delay: Duration::from_millis(100),
//!             max_delay: Duration::from_secs(10),
//!             max_retries: 3,
//!             jitter: true,
//!         })
//!         .build()?;
//!
//!     // Make a GET request
//!     let user = client.get::<User>("/users/123").await?;
//!     println!("User: {}", user.data.name);
//!     println!("Request took {:?}", user.latency);
//!
//!     // Make a POST request
//!     let new_user = CreateUser {
//!         name: "Alice".to_string(),
//!         email: "alice@example.com".to_string(),
//!     };
//!     let created = client.post::<_, User>("/users", &new_user).await?;
//!     println!("Created user with ID: {}", created.data.id);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - **Type-safe requests and responses** - Generic over request/response types with automatic JSON serialization
//! - **Rich error handling** - Comprehensive error types with access to raw responses and HTTP details
//! - **Flexible retry logic** - Exponential backoff, linear, or custom retry strategies
//! - **Customizable retry predicates** - Retry on 5xx, timeouts, network errors, or custom conditions
//! - **Automatic logging** - Structured logging with `tracing` for observability
//! - **Response metadata** - Access latency, status codes, headers, retry attempts, and raw response bodies
//! - **Builder pattern** - Fluent API for configuring clients
//! - **Connection pooling** - Reusable clients with efficient connection management
//!
//! ## Error Handling
//!
//! Calleen provides detailed error information while preserving raw response data:
//!
//! ```no_run
//! use calleen::{Client, Error};
//!
//! # async fn example() -> Result<(), Error> {
//! # let client = Client::builder().base_url("https://api.example.com")?.build()?;
//! match client.get::<serde_json::Value>("/endpoint").await {
//!     Ok(response) => {
//!         println!("Success: {:?}", response.data);
//!     }
//!     Err(Error::DeserializationFailed { raw_response, serde_error, status }) => {
//!         eprintln!("Failed to deserialize (status {}):", status);
//!         eprintln!("  Raw response: {}", raw_response);
//!         eprintln!("  Error: {}", serde_error);
//!     }
//!     Err(Error::HttpError { status, raw_response, .. }) => {
//!         eprintln!("HTTP error {}: {}", status, raw_response);
//!     }
//!     Err(e) => {
//!         eprintln!("Other error: {}", e);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Retry Strategies
//!
//! Configure how the client handles transient failures:
//!
//! ```no_run
//! use calleen::{Client, RetryStrategy, retry::{RetryOn5xx, RetryOnTimeout, OrPredicate}};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), calleen::Error> {
//! let client = Client::builder()
//!     .base_url("https://api.example.com")?
//!     .retry_strategy(RetryStrategy::ExponentialBackoff {
//!         initial_delay: Duration::from_millis(100),
//!         max_delay: Duration::from_secs(30),
//!         max_retries: 5,
//!         jitter: true, // Recommended to prevent thundering herd
//!     })
//!     .retry_predicate(Box::new(OrPredicate::new(vec![
//!         Box::new(RetryOn5xx),
//!         Box::new(RetryOnTimeout),
//!     ])))
//!     .build()?;
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
pub mod metadata;
mod response;
pub mod retry;

pub use client::{Client, ClientBuilder};
pub use error::{Error, Result};
pub use response::Response;
pub use retry::{RetryPredicate, RetryStrategy};
