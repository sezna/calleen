//! Response wrapper that preserves both parsed data and raw response details.
//!
//! The [`Response`] type wraps the deserialized response data along with metadata
//! about the HTTP request, making it easy to access timing information, headers,
//! and the raw response body for debugging and observability.

use http::{HeaderMap, StatusCode};
use std::time::Duration;

/// A wrapper around a successful HTTP response.
///
/// This type provides both the deserialized response data and metadata about
/// the HTTP transaction, including latency, status code, headers, and the raw
/// response body.
///
/// # Type Parameters
///
/// * `T` - The type of the deserialized response data
///
/// # Examples
///
/// ```no_run
/// use calleen::Client;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct User {
///     id: u64,
///     name: String,
/// }
///
/// # async fn example() -> Result<(), calleen::Error> {
/// let client = Client::builder()
///     .base_url("https://api.example.com")?
///     .build()?;
///
/// let response = client.get::<User>("/users/123").await?;
///
/// println!("User: {}", response.data.name);
/// println!("Request took {:?}", response.latency);
/// println!("Status: {}", response.status);
/// println!("Retry attempts: {}", response.attempts);
///
/// // Access raw response for debugging
/// if response.latency > std::time::Duration::from_secs(1) {
///     println!("Slow response body: {}", response.raw_body);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Response<T> {
    /// The deserialized response data.
    pub data: T,

    /// The raw response body as a string.
    ///
    /// This is useful for debugging, logging, or when you need to inspect
    /// the exact response from the server.
    pub raw_body: String,

    /// The HTTP status code of the response.
    pub status: StatusCode,

    /// The response headers.
    pub headers: HeaderMap,

    /// The total latency of the request, including all retry attempts.
    ///
    /// This measures the time from when the first request was sent until
    /// the successful response was received.
    pub latency: Duration,

    /// The number of attempts made to complete this request.
    ///
    /// This will be `1` for requests that succeeded on the first try,
    /// and higher for requests that required retries.
    pub attempts: usize,
}

impl<T> Response<T> {
    /// Creates a new `Response`.
    ///
    /// This is typically called internally by the client after successfully
    /// deserializing a response.
    pub fn new(
        data: T,
        raw_body: String,
        status: StatusCode,
        headers: HeaderMap,
        latency: Duration,
        attempts: usize,
    ) -> Self {
        Self {
            data,
            raw_body,
            status,
            headers,
            latency,
            attempts,
        }
    }

    /// Maps the response data to a different type using the provided function.
    ///
    /// This is useful when you want to transform the response data while
    /// preserving the metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// # use calleen::Response;
    /// # use http::{HeaderMap, StatusCode};
    /// # use std::time::Duration;
    /// let response = Response::new(
    ///     42,
    ///     "42".to_string(),
    ///     StatusCode::OK,
    ///     HeaderMap::new(),
    ///     Duration::from_millis(100),
    ///     1,
    /// );
    ///
    /// let string_response = response.map(|n| n.to_string());
    /// assert_eq!(string_response.data, "42");
    /// ```
    pub fn map<U, F>(self, f: F) -> Response<U>
    where
        F: FnOnce(T) -> U,
    {
        Response {
            data: f(self.data),
            raw_body: self.raw_body,
            status: self.status,
            headers: self.headers,
            latency: self.latency,
            attempts: self.attempts,
        }
    }

    /// Returns `true` if the request required retries.
    ///
    /// # Examples
    ///
    /// ```
    /// # use calleen::Response;
    /// # use http::{HeaderMap, StatusCode};
    /// # use std::time::Duration;
    /// let response = Response::new(
    ///     (),
    ///     String::new(),
    ///     StatusCode::OK,
    ///     HeaderMap::new(),
    ///     Duration::from_millis(100),
    ///     3,
    /// );
    ///
    /// assert!(response.was_retried());
    /// ```
    pub fn was_retried(&self) -> bool {
        self.attempts > 1
    }

    /// Returns a reference to a header value by name.
    ///
    /// # Examples
    ///
    /// ```
    /// # use calleen::Response;
    /// # use http::{HeaderMap, StatusCode, HeaderValue};
    /// # use std::time::Duration;
    /// let mut headers = HeaderMap::new();
    /// headers.insert("content-type", HeaderValue::from_static("application/json"));
    ///
    /// let response = Response::new(
    ///     (),
    ///     String::new(),
    ///     StatusCode::OK,
    ///     headers,
    ///     Duration::from_millis(100),
    ///     1,
    /// );
    ///
    /// assert_eq!(
    ///     response.header("content-type").unwrap(),
    ///     "application/json"
    /// );
    /// ```
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}

impl<T> AsRef<T> for Response<T> {
    fn as_ref(&self) -> &T {
        &self.data
    }
}

impl<T> std::ops::Deref for Response<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
