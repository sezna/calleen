//! Error types for HTTP API calls.
//!
//! This module provides comprehensive error types that preserve maximum debugging information
//! while remaining ergonomic to use. All errors include context about what went wrong and
//! provide access to raw response data when available.

use http::{HeaderMap, StatusCode};

/// The main error type for HTTP API calls.
///
/// This error type preserves all relevant debugging information including raw responses,
/// HTTP status codes, headers, and underlying error details.
///
/// # Examples
///
/// ```no_run
/// use calleen::{Client, Error};
///
/// # async fn example() -> Result<(), Error> {
/// let client = Client::builder()
///     .base_url("https://api.example.com")?
///     .build()?;
///
/// match client.get::<serde_json::Value>("/endpoint").await {
///     Ok(response) => println!("Success: {:?}", response.data),
///     Err(Error::DeserializationFailed { raw_response, serde_error, .. }) => {
///         eprintln!("Failed to deserialize. Raw response: {}", raw_response);
///         eprintln!("Serde error: {}", serde_error);
///     }
///     Err(Error::HttpError { status, raw_response, .. }) => {
///         eprintln!("HTTP error {}: {}", status, raw_response);
///     }
///     Err(e) => eprintln!("Other error: {}", e),
/// }
/// # Ok(())
/// # }
/// ```
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// A network-level error occurred (connection failed, DNS lookup failed, etc.).
    ///
    /// This wraps the underlying `reqwest::Error` and indicates problems at the network layer
    /// rather than the HTTP protocol layer.
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The request timed out.
    ///
    /// This occurs when the request takes longer than the configured timeout duration.
    #[error("Request timed out")]
    Timeout,

    /// Failed to deserialize the response body into the expected type.
    ///
    /// This error preserves both the raw response text and the serde error message,
    /// making it easy to debug deserialization issues in production.
    ///
    /// # Fields
    ///
    /// * `raw_response` - The raw response body as a string
    /// * `serde_error` - The error message from serde
    /// * `status` - The HTTP status code of the response
    #[error("Failed to deserialize response (status {status}): {serde_error}")]
    DeserializationFailed {
        /// The raw response body that failed to deserialize
        raw_response: String,
        /// The serde error message
        serde_error: String,
        /// The HTTP status code
        status: StatusCode,
    },

    /// The server returned a non-2xx HTTP status code.
    ///
    /// This error includes the full response details for debugging.
    ///
    /// # Fields
    ///
    /// * `status` - The HTTP status code
    /// * `raw_response` - The raw response body
    /// * `headers` - The response headers
    #[error("HTTP error {status}: {raw_response}")]
    HttpError {
        /// The HTTP status code
        status: StatusCode,
        /// The raw response body
        raw_response: String,
        /// The response headers
        headers: HeaderMap,
    },

    /// Invalid configuration was provided.
    ///
    /// This indicates a problem with how the client or request was configured,
    /// such as an invalid URL or invalid header values.
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Maximum number of retries was exceeded.
    ///
    /// This error is returned when all retry attempts have been exhausted.
    /// It includes the number of attempts made and the last error encountered.
    ///
    /// # Fields
    ///
    /// * `attempts` - The number of retry attempts made
    /// * `last_error` - The last error encountered before giving up
    #[error("Max retries exceeded after {attempts} attempts: {last_error}")]
    MaxRetriesExceeded {
        /// The number of attempts made
        attempts: usize,
        /// The last error encountered
        last_error: Box<Error>,
    },

    /// Failed to serialize the request body.
    ///
    /// This occurs when the request body cannot be serialized to JSON.
    #[error("Failed to serialize request: {0}")]
    SerializationFailed(String),

    /// An invalid URL was provided.
    ///
    /// This wraps URL parsing errors.
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
}

impl Error {
    /// Returns `true` if this error is potentially retryable.
    ///
    /// Network errors, timeouts, and 5xx HTTP errors are considered retryable.
    /// 4xx errors and deserialization failures are not.
    ///
    /// # Examples
    ///
    /// ```
    /// use calleen::Error;
    /// use http::StatusCode;
    ///
    /// let err = Error::HttpError {
    ///     status: StatusCode::INTERNAL_SERVER_ERROR,
    ///     raw_response: "Server error".to_string(),
    ///     headers: http::HeaderMap::new(),
    /// };
    ///
    /// assert!(err.is_retryable());
    ///
    /// let err = Error::HttpError {
    ///     status: StatusCode::BAD_REQUEST,
    ///     raw_response: "Bad request".to_string(),
    ///     headers: http::HeaderMap::new(),
    /// };
    ///
    /// assert!(!err.is_retryable());
    /// ```
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Network(_) => true,
            Error::Timeout => true,
            Error::HttpError { status, .. } => status.is_server_error(),
            Error::DeserializationFailed { .. } => false,
            Error::ConfigurationError(_) => false,
            Error::MaxRetriesExceeded { .. } => false,
            Error::SerializationFailed(_) => false,
            Error::InvalidUrl(_) => false,
        }
    }

    /// Returns the HTTP status code if this error has one.
    ///
    /// Returns `Some(status)` for `HttpError` and `DeserializationFailed` errors,
    /// `None` for other error types.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Error::HttpError { status, .. } => Some(*status),
            Error::DeserializationFailed { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Returns the raw response body if this error has one.
    ///
    /// Returns `Some(&str)` for errors that include response bodies,
    /// `None` for other error types.
    pub fn raw_response(&self) -> Option<&str> {
        match self {
            Error::HttpError { raw_response, .. } => Some(raw_response),
            Error::DeserializationFailed { raw_response, .. } => Some(raw_response),
            _ => None,
        }
    }
}

/// A specialized `Result` type for HTTP API calls.
///
/// This is a convenience alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
