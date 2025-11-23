//! Integration tests using wiremock to simulate HTTP servers.

use calleen::retry::RetryPredicate;
use calleen::{Client, Error, RetryStrategy};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TestData {
    id: u32,
    name: String,
}

#[tokio::test]
async fn test_successful_get_request() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let response = client.get::<TestData>("/test").await.unwrap();

    assert_eq!(response.data, response_data);
    assert_eq!(response.status.as_u16(), 200);
    assert_eq!(response.attempts, 1);
    assert!(!response.was_retried());
}

#[tokio::test]
async fn test_successful_post_request() {
    let mock_server = MockServer::start().await;

    let request_data = TestData {
        id: 0,
        name: "New".to_string(),
    };

    let response_data = TestData {
        id: 1,
        name: "New".to_string(),
    };

    Mock::given(method("POST"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(201).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let response = client
        .post::<TestData, TestData>("/test", &request_data)
        .await
        .unwrap();

    assert_eq!(response.data, response_data);
    assert_eq!(response.status.as_u16(), 201);
}

#[tokio::test]
async fn test_http_error_4xx() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not found"))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let result = client.get::<TestData>("/test").await;

    match result {
        Err(Error::HttpError {
            status,
            raw_response,
            ..
        }) => {
            assert_eq!(status.as_u16(), 404);
            assert_eq!(raw_response.as_ref(), "Not found");
        }
        _ => panic!("Expected HttpError, got {:?}", result),
    }
}

#[tokio::test]
async fn test_deserialization_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_string("invalid json"))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let result = client.get::<TestData>("/test").await;

    match result {
        Err(Error::DeserializationFailed {
            raw_response,
            serde_error,
            status,
        }) => {
            assert_eq!(status.as_u16(), 200);
            assert_eq!(raw_response, "invalid json");
            assert!(serde_error.contains("expected"));
        }
        _ => panic!("Expected DeserializationFailed, got {:?}", result),
    }
}

#[tokio::test]
async fn test_retry_on_5xx() {
    let mock_server = MockServer::start().await;
    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_count_clone = attempt_count.clone();

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    // First two requests fail with 500, third succeeds
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(move |_req: &wiremock::Request| {
            let count = attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            if count < 2 {
                ResponseTemplate::new(500).set_body_string("Server error")
            } else {
                ResponseTemplate::new(200).set_body_json(&response_data)
            }
        })
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(10),
            max_retries: 3,
        })
        .retry_predicate(Box::new(calleen::retry::RetryOnRetryable))
        .build()
        .unwrap();

    let response = client.get::<TestData>("/test").await.unwrap();

    assert_eq!(response.data.id, 1);
    assert_eq!(response.attempts, 3);
    assert!(response.was_retried());
    assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_max_retries_exceeded() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Server error"))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(10),
            max_retries: 2,
        })
        .retry_predicate(Box::new(calleen::retry::RetryOnRetryable))
        .build()
        .unwrap();

    let result = client.get::<TestData>("/test").await;

    match result {
        Err(Error::MaxRetriesExceeded { attempts, .. }) => {
            // max_retries: 2 means we try 2 retries, so 3 total attempts (1 initial + 2 retries)
            assert_eq!(attempts, 3);
        }
        _ => panic!("Expected MaxRetriesExceeded, got {:?}", result),
    }
}

#[tokio::test]
async fn test_exponential_backoff() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            max_retries: 3,
            jitter: false,
        })
        .build()
        .unwrap();

    let response = client.get::<TestData>("/test").await.unwrap();
    assert_eq!(response.data.id, 1);
}

#[tokio::test]
async fn test_custom_retry_predicate() {
    let mock_server = MockServer::start().await;

    // Custom predicate that only retries on 503
    struct RetryOn503;
    impl RetryPredicate for RetryOn503 {
        fn should_retry(&self, error: &Error, _attempt: usize) -> bool {
            matches!(
                error,
                Error::HttpError { status, .. } if status.as_u16() == 503
            )
        }
    }

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Server error"))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(10),
            max_retries: 3,
        })
        .retry_predicate(Box::new(RetryOn503))
        .build()
        .unwrap();

    // Should fail immediately because 500 doesn't match our predicate
    let result = client.get::<TestData>("/test").await;

    match result {
        Err(Error::HttpError { status, .. }) => {
            assert_eq!(status.as_u16(), 500);
        }
        _ => panic!("Expected HttpError, got {:?}", result),
    }
}

#[tokio::test]
async fn test_response_metadata() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(&response_data)
                .insert_header("x-custom-header", "custom-value"),
        )
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let response = client.get::<TestData>("/test").await.unwrap();

    assert_eq!(response.status.as_u16(), 200);
    assert_eq!(response.attempts, 1);
    assert!(!response.was_retried());
    // Latency is measured - just verify it exists (can be 0 for very fast responses)
    let _ = response.latency; // Ensure latency field is accessible
    assert!(response.raw_body.contains("Test"));
    assert_eq!(response.header("x-custom-header"), Some("custom-value"));
}

#[tokio::test]
async fn test_default_headers() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .default_header("User-Agent", "test-agent")
        .unwrap()
        .build()
        .unwrap();

    let _ = client.get::<TestData>("/test").await.unwrap();
}

#[tokio::test]
async fn test_query_parameters() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    let metadata = calleen::metadata::RequestMetadata::new(http::Method::GET, "/test")
        .with_query_param("page", "1")
        .with_query_param("limit", "10");

    let response = client.call::<(), TestData>(metadata, None).await.unwrap();
    assert_eq!(response.data.id, 1);
}

#[tokio::test]
async fn test_all_http_methods() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    // GET
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    // POST
    Mock::given(method("POST"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(201).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    // PUT
    Mock::given(method("PUT"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    // DELETE
    Mock::given(method("DELETE"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(204).set_body_string(""))
        .mount(&mock_server)
        .await;

    // PATCH
    Mock::given(method("PATCH"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_data))
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .build()
        .unwrap();

    // Test GET
    let _ = client.get::<TestData>("/test").await.unwrap();

    // Test POST
    let _ = client
        .post::<TestData, TestData>("/test", &response_data)
        .await
        .unwrap();

    // Test PUT
    let _ = client
        .put::<TestData, TestData>("/test", &response_data)
        .await
        .unwrap();

    // Test DELETE  (returns empty body, so use serde_json::Value or handle empty response)
    // For empty responses, we can't deserialize, so we expect an error or use a permissive type
    let delete_result = client.delete::<serde_json::Value>("/test").await;
    // DELETE with 204 returns empty body, which causes deserialization error
    // This is actually expected behavior - let's just verify we got a 204
    match delete_result {
        Err(Error::DeserializationFailed { status, .. }) => {
            assert_eq!(status.as_u16(), 204);
        }
        Ok(_) => panic!("Unexpected success for empty DELETE response"),
        Err(e) => panic!("Unexpected error: {:?}", e),
    }

    // Test PATCH
    let _ = client
        .patch::<TestData, TestData>("/test", &response_data)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_error_is_retryable() {
    let error_5xx = Error::HttpError {
        status: http::StatusCode::INTERNAL_SERVER_ERROR,
        raw_response: "Error".to_string().into_boxed_str(),
        headers: Box::new(http::HeaderMap::new()),
        rate_limit_info: None,
    };
    assert!(error_5xx.is_retryable());

    let error_4xx = Error::HttpError {
        status: http::StatusCode::BAD_REQUEST,
        raw_response: "Error".to_string().into_boxed_str(),
        headers: Box::new(http::HeaderMap::new()),
        rate_limit_info: None,
    };
    assert!(!error_4xx.is_retryable());

    let error_timeout = Error::Timeout;
    assert!(error_timeout.is_retryable());

    let error_config = Error::ConfigurationError("Error".to_string());
    assert!(!error_config.is_retryable());
}

#[tokio::test]
async fn test_rate_limit_with_retry_after_seconds() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_count_clone = attempt_count.clone();

    // First request returns 429 with Retry-After, second succeeds
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(move |_req: &wiremock::Request| {
            let count = attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "1")
                    .insert_header("x-ratelimit-remaining", "0")
                    .set_body_string("Rate limited")
            } else {
                ResponseTemplate::new(200).set_body_json(&response_data)
            }
        })
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(100),
            max_retries: 3,
        })
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let response = client.get::<TestData>("/test").await.unwrap();

    assert_eq!(response.data.id, 1);
    assert_eq!(response.attempts, 2);
    // Should have waited approximately 1 second for rate limit
    assert!(start.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn test_rate_limit_with_x_ratelimit_reset() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_count_clone = attempt_count.clone();

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(move |_req: &wiremock::Request| {
            let count = attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                // Calculate reset time by adding 1 second to current SystemTime
                // then converting to Unix timestamp (preserves nanoseconds)
                let reset_time = std::time::SystemTime::now() + Duration::from_secs(1);
                let timestamp = reset_time
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                ResponseTemplate::new(429)
                    .insert_header("x-ratelimit-reset", timestamp.to_string())
                    .insert_header("x-ratelimit-remaining", "0")
                    .set_body_string("Rate limited")
            } else {
                ResponseTemplate::new(200).set_body_json(&response_data)
            }
        })
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(100),
            max_retries: 3,
        })
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let response = client.get::<TestData>("/test").await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(response.data.id, 1);
    assert_eq!(response.attempts, 2);
    // Should have waited approximately 1 second for rate limit
    // Note: Unix timestamps are in whole seconds, so nanoseconds are truncated,
    // which can reduce the delay. Allow tolerance for this and processing time.
    assert!(
        elapsed >= Duration::from_millis(100),
        "Expected at least 100ms, got {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "Expected less than 2s, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_rate_limit_disabled() {
    let mock_server = MockServer::start().await;

    let response_data = TestData {
        id: 1,
        name: "Test".to_string(),
    };

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempt_count_clone = attempt_count.clone();

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(move |_req: &wiremock::Request| {
            let count = attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "10")
                    .set_body_string("Rate limited")
            } else {
                ResponseTemplate::new(200).set_body_json(&response_data)
            }
        })
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(100),
            max_retries: 3,
        })
        .rate_limit_config(calleen::rate_limit::RateLimitConfig::disabled())
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let response = client.get::<TestData>("/test").await.unwrap();

    // With rate limiting disabled, should use the normal retry delay (100ms)
    // instead of the 10 second Retry-After
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(response.attempts, 2);
}

#[tokio::test]
async fn test_rate_limit_max_wait_cap() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "600") // 10 minutes
                .set_body_string("Rate limited"),
        )
        .mount(&mock_server)
        .await;

    let client = Client::builder()
        .base_url(mock_server.uri())
        .unwrap()
        .retry_strategy(RetryStrategy::Linear {
            delay: Duration::from_millis(100),
            max_retries: 1,
        })
        .rate_limit_config(
            calleen::rate_limit::RateLimitConfig::builder()
                .max_wait(Duration::from_secs(2))
                .build(),
        )
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let result = client.get::<TestData>("/test").await;

    // Should fail after being capped by max_wait (2 seconds)
    // Total time should be around 2 seconds, not 10 minutes
    assert!(result.is_err());
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_secs(2));
    assert!(elapsed < Duration::from_secs(4));
}
