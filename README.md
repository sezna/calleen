# Calleen

A robust, type-safe HTTP API client library for Rust with rich error handling, automatic retries, and comprehensive observability.

## Features

- **Type-safe requests and responses** - Generic over request/response types with automatic JSON serialization
- **Rich error handling** - Comprehensive error types with access to raw responses and HTTP details
- **Flexible retry logic** - Exponential backoff, linear, or custom retry strategies
- **Customizable retry predicates** - Retry on 5xx, timeouts, network errors, or custom conditions
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
```

## Why Calleen?

### vs. Raw `reqwest`

| Feature | Calleen | reqwest |
|---------|---------|---------|
| Automatic retries | ✅ | ❌ |
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

## Future Roadmap

- [ ] Rate limiting support
- [ ] Circuit breaker pattern
- [ ] Request/response middleware chain
- [ ] Metrics collection hooks
- [ ] Mock mode for testing
- [ ] Connection pooling configuration
- [ ] Support for other serialization formats (XML, protobuf)
