# Calleen

[![Crates.io](https://img.shields.io/crates/v/calleen.svg)](https://crates.io/crates/calleen)
[![Documentation](https://docs.rs/calleen/badge.svg)](https://docs.rs/calleen)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

I've been writing production Rust applications for quite a few years now, and in every new project I find myself replicating
certain patterns. This library provides what I would consider "best practices" when sending an HTTP request, and parsing its response.

To avoid [the XY problem](https://xyproblem.info/), let me first describe the problems I wanted to solve:
1. `serde`/`serde_json` don't retain the raw data when they fail to deserialize. This means that you'll get error logs that say "failed to deserialize" but have no insight into what the bad input was. As an individual, it is easy to work around this. But, as a team, it slips through pretty frequently, especially with engineers new to Rust and on-call log debugging.
2. Retry logic built in to the call layer that is HTTP-response-code aware -- I've been in many projects where we have ad-hoc retry logic at the callsite. And sometimes it knows not to retry e.g. 4xx errors, and only to retry 5xx errors. Sometimes it doesn't know. 
3. Critical failures and non-actionable were not disambiguated, meaning you could get paged when on-call for a third party 5xx response. Something you as an engineer can do nothing about!

This library addresses these three concerns primarily.
1. `calleen` retains the raw response, so if deserialization fails, the error log contains the raw input. This does have some memory overhead, but _it is worth it_. As somebody who has been paged at 1am for a serde deserialization failure many times in his life, I will always spend these bytes.
2. Centralized retry strategy definitions which are status-code aware and reasonably customizable.
3. Disambiguation among various failure modes -- `tracing::warn!()` on typically non-actionable responses like 5xx, `tracing::error!()` on `4xx` or failure to deserialize response types, which are typically actionable and urgent. For companies I've worked in, we typically page on `error!()` logs, so this triggers our PagerDuty.

## Features

- **Type-safe requests and responses** - Generic over request/response types with automatic JSON serialization
- **Rich error handling** - Comprehensive error types with access to raw responses and HTTP details
- **Flexible retry logic** - Exponential backoff, linear, or custom retry strategies
- **Customizable retry predicates** - Retry on 5xx, timeouts, network errors, or custom conditions
- **Stateful rate limiting** - Automatic parsing and respect for rate limit headers (Retry-After, X-RateLimit-*)
- **Automatic logging** - Structured logging with `tracing` for observability
- **Response metadata** - Access latency, status codes, headers, retry attempts, and raw response bodies
- **Builder pattern** - Fluent API for configuring clients
- **Connection pooling** - Reusable clients with efficient connection management

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
calleen = "0.1"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

## Quick Start

```rust
use calleen::{Client, RetryStrategy};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

#[tokio::main]
async fn main() -> Result<(), calleen::Error> {
    // Create a client with retry logic
    let client = Client::builder()
        .base_url("https://api.example.com")?
        .timeout(Duration::from_secs(30))
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            max_retries: 3,
            jitter: true,
        })
        .build()?;

    // Make a GET request
    let user = client.get::<User>("/users/123").await?;
    println!("User: {}", user.data.name);
    println!("Request took {:?}", user.latency);

    // Make a POST request
    let new_user = CreateUser {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };
    let created = client.post::<_, User>("/users", &new_user).await?;
    println!("Created user with ID: {}", created.data.id);

    Ok(())
}
```

## Error Handling

Calleen provides detailed error information while preserving raw response data for debugging:

```rust
use calleen::{Client, Error};

match client.get::<User>("/users/123").await {
    Ok(response) => {
        println!("User: {:?}", response.data);
    }
    Err(Error::DeserializationFailed { raw_response, serde_error, status }) => {
        eprintln!("Failed to deserialize (status {}):", status);
        eprintln!("  Raw response: {}", raw_response);
        eprintln!("  Error: {}", serde_error);
    }
    Err(Error::HttpError { status, raw_response, .. }) => {
        eprintln!("HTTP error {}: {}", status, raw_response);
    }
    Err(Error::Timeout) => {
        eprintln!("Request timed out");
    }
    Err(Error::Network(e)) => {
        eprintln!("Network error: {}", e);
    }
    Err(e) => {
        eprintln!("Other error: {}", e);
    }
}
```

### Error Types

- `Network(reqwest::Error)` - Network-level errors (connection failed, DNS, etc.)
- `Timeout` - Request timeout
- `DeserializationFailed { raw_response, serde_error, status }` - Failed to parse response
- `HttpError { status, raw_response, headers }` - Non-2xx HTTP status
- `ConfigurationError(String)` - Invalid client configuration
- `MaxRetriesExceeded { attempts, last_error }` - All retry attempts exhausted
- `SerializationFailed(String)` - Failed to serialize request body
- `InvalidUrl(url::ParseError)` - Invalid URL

## Retry Strategies

### Exponential Backoff (Recommended)

```rust
use calleen::{Client, RetryStrategy};
use std::time::Duration;

let client = Client::builder()
    .base_url("https://api.example.com")?
    .retry_strategy(RetryStrategy::ExponentialBackoff {
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_secs(30),
        max_retries: 5,
        jitter: true, // Recommended to prevent thundering herd
    })
    .build()?;
```

Delays: 100ms, 200ms, 400ms, 800ms, 1600ms... (with random jitter)

### Linear Backoff

```rust
let client = Client::builder()
    .base_url("https://api.example.com")?
    .retry_strategy(RetryStrategy::Linear {
        delay: Duration::from_secs(1),
        max_retries: 3,
    })
    .build()?;
```

Delays: 1s, 1s, 1s...

### Custom Retry Logic

```rust
let client = Client::builder()
    .base_url("https://api.example.com")?
    .retry_strategy(RetryStrategy::Custom {
        delay_fn: |attempt| match attempt {
            1 => Some(Duration::from_millis(100)),
            2 => Some(Duration::from_millis(500)),
            3 => Some(Duration::from_secs(2)),
            _ => None,
        },
    })
    .build()?;
```

## Custom Retry Predicates

Control when to retry based on error type, status code, or custom logic:

```rust
use calleen::retry::{RetryPredicate, RetryOn5xx, RetryOnTimeout, OrPredicate};

// Retry on rate limit errors (HTTP 429)
struct RetryOnRateLimit;

impl RetryPredicate for RetryOnRateLimit {
    fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
        matches!(
            error,
            Error::HttpError { status, .. } if status.as_u16() == 429
        )
    }
}

// Combine predicates: retry on 5xx OR timeouts OR rate limits
let client = Client::builder()
    .base_url("https://api.example.com")?
    .retry_predicate(Box::new(OrPredicate::new(vec![
        Box::new(RetryOn5xx),
        Box::new(RetryOnTimeout),
        Box::new(RetryOnRateLimit),
    ])))
    .build()?;
```

### Built-in Predicates

- `RetryOnRetryable` - Retry on network errors, timeouts, and 5xx errors (default)
- `RetryOn5xx` - Retry only on 5xx server errors
- `RetryOnTimeout` - Retry only on timeout errors
- `RetryOnConnectionError` - Retry only on network/connection errors
- `OrPredicate` - Combine predicates with OR logic
- `AndPredicate` - Combine predicates with AND logic

## Rate Limiting

Calleen automatically parses and respects rate limit headers from API responses. When enabled (default), rate limit delays take precedence over normal retry delays.

### Automatic Rate Limit Handling

```rust
use calleen::{Client, RetryStrategy};
use std::time::Duration;

// Rate limiting is enabled by default
let client = Client::builder()
    .base_url("https://api.example.com")?
    .retry_strategy(RetryStrategy::ExponentialBackoff {
        initial_delay: Duration::from_millis(100),
        max_delay: Duration::from_secs(30),
        max_retries: 5,
        jitter: true,
    })
    .build()?;

// Automatically respects Retry-After and X-RateLimit-* headers
let response = client.get::<User>("/users/123").await?;
```

### Supported Headers

Calleen parses the following standard rate limit headers:

- `Retry-After` - Delay in seconds or HTTP date
- `X-RateLimit-Reset` - Unix timestamp when rate limit resets
- `X-RateLimit-Remaining` - Number of requests remaining in current window

### Custom Rate Limit Configuration

```rust
use calleen::rate_limit::RateLimitConfig;
use std::time::Duration;

let client = Client::builder()
    .base_url("https://api.example.com")?
    .rate_limit_config(
        RateLimitConfig::builder()
            .enabled(true)
            .max_wait(Duration::from_secs(120)) // Cap rate limit waits at 2 minutes
            .respect_retry_after(true)
            .build(),
    )
    .build()?;
```

### Disabling Rate Limiting

```rust
use calleen::rate_limit::RateLimitConfig;

let client = Client::builder()
    .base_url("https://api.example.com")?
    .rate_limit_config(RateLimitConfig::builder().enabled(false).build())
    .build()?;
```

### Accessing Rate Limit Information

```rust
use calleen::Error;

match client.get::<User>("/users/123").await {
    Ok(response) => println!("Success: {:?}", response.data),
    Err(e) => {
        // Check for rate limit information
        if let Some(info) = e.rate_limit_info() {
            println!("Rate limited!");
            if let Some(retry_after) = info.retry_after {
                println!("  Retry after: {:?}", retry_after);
            }
            if let Some(reset_at) = info.reset_at {
                println!("  Reset at: {:?}", reset_at);
            }
            if let Some(remaining) = info.remaining {
                println!("  Remaining: {}", remaining);
            }

            // Get recommended delay with custom max wait
            if let Some(delay) = e.rate_limit_delay(Duration::from_secs(60)) {
                println!("  Recommended delay: {:?}", delay);
            }
        }
    }
}
```

### Rate Limit Behavior

When a rate limit is encountered (HTTP 429 or rate limit headers present):

1. Rate limit delay from `Retry-After` header is preferred over normal retry strategy
2. Delay is capped by the configured `max_wait` (default: 5 minutes)
3. If no rate limit info is available, normal retry strategy applies
4. Jitter is still added to prevent thundering herd
5. Rate limit errors are logged with `tracing::warn!` for visibility

## Response Metadata

Access detailed information about each request:

```rust
let response = client.get::<User>("/users/123").await?;

println!("Data: {:?}", response.data);
println!("Latency: {:?}", response.latency);
println!("Status: {}", response.status);
println!("Attempts: {}", response.attempts);
println!("Was retried: {}", response.was_retried());
println!("Content-Type: {:?}", response.header("content-type"));
println!("Raw body (first 100 chars): {}",
         response.raw_body.chars().take(100).collect::<String>());
```

## Advanced Usage

### Custom Headers

```rust
let client = Client::builder()
    .base_url("https://api.example.com")?
    .default_header("User-Agent", "my-app/1.0")?
    .default_header("Authorization", "Bearer token")?
    .build()?;
```

### Request Metadata

```rust
use calleen::metadata::RequestMetadata;
use http::Method;

let metadata = RequestMetadata::new(Method::POST, "/users")
    .with_header("X-Custom-Header", "value")?
    .with_query_param("page", "1")
    .with_query_param("limit", "10");

let response = client.call::<_, User>(metadata, Some(&request_body)).await?;
```

### All HTTP Methods

```rust
// GET
let response = client.get::<User>("/users/123").await?;

// POST
let response = client.post::<CreateUser, User>("/users", &new_user).await?;

// PUT
let response = client.put::<UpdateUser, User>("/users/123", &update).await?;

// DELETE
let response = client.delete::<()>("/users/123").await?;

// PATCH
let response = client.patch::<PatchUser, User>("/users/123", &patch).await?;
```

## Logging

Calleen uses the `tracing` crate for structured logging. Initialize a subscriber to see logs:

```rust
tracing_subscriber::fmt()
    .with_env_filter("calleen=debug")
    .init();
```

Log levels:
- `debug` - Request details, serialization
- `info` - Response received, latency
- `warn` - Retries, 5xx errors
- `error` - 4xx errors, deserialization failures

## Examples

Run the examples to see Calleen in action:

```bash
# Basic GET and POST requests
cargo run --example basic_call

# Different retry strategies
cargo run --example retry_strategies

# Comprehensive error handling
cargo run --example error_handling

# Custom retry predicates
cargo run --example custom_retry

# Rate limiting features
cargo run --example rate_limiting
```

## Why Calleen?

### vs. Raw `reqwest`

| Feature | Calleen | reqwest |
|---------|---------|---------|
| Automatic retries | ✅ | ❌ |
| Stateful rate limiting | ✅ | ❌ |
| Rich error types with raw response | ✅ | ❌ |
| Built-in logging | ✅ | ❌ |
| Response metadata (latency, attempts) | ✅ | ❌ |
| Type-safe requests/responses | ✅ | ✅ |
| Connection pooling | ✅ | ✅ |

Calleen builds on top of `reqwest` to provide a higher-level, more production-ready API client experience.

## Design Philosophy

1. **Preserve debugging information** - Always keep raw responses, error messages, and metadata
2. **Type safety** - Leverage Rust's type system for compile-time guarantees
3. **Sensible defaults** - Works out of the box, configurable when needed
4. **Composability** - Retry predicates, strategies, and headers are all composable
5. **Observability** - Built-in logging and metrics-friendly design

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Comparison with Similar Libraries

- **reqwest** - Low-level HTTP client. Calleen builds on reqwest with retries and error handling.
- **surf** - Async HTTP client with middleware. Calleen focuses on retry logic and error context.
- **ureq** - Synchronous HTTP client. Calleen is async-first with tokio.
