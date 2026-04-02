//! HTTP client with retry logic and rich error handling.
//!
//! The [`Client`] type is the main entry point for making HTTP requests.
//! Use [`ClientBuilder`] to configure and create clients.

use crate::{
    metadata::RequestMetadata,
    rate_limit::RateLimitConfig,
    retry::{RetryOnRetryable, RetryPredicate, RetryStrategy},
    Error, Response, Result,
};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

/// An HTTP client for making API calls with retry logic and rich error handling.
///
/// The client is designed to be reused across multiple requests. It maintains
/// a connection pool and configuration that applies to all requests.
///
/// # Examples
///
/// ```no_run
/// use calleen::{Client, Response, RetryStrategy};
/// use std::time::Duration;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize)]
/// struct CreateUser {
///     name: String,
///     email: String,
/// }
///
/// #[derive(Deserialize)]
/// struct User {
///     id: u64,
///     name: String,
///     email: String,
/// }
///
/// # async fn example() -> Result<(), calleen::Error> {
/// let client = Client::builder()
///     .base_url("https://api.example.com")?
///     .timeout(Duration::from_secs(30))
///     .retry_strategy(RetryStrategy::ExponentialBackoff {
///         initial_delay: Duration::from_millis(100),
///         max_delay: Duration::from_secs(10),
///         max_retries: 3,
///         jitter: true,
///     })
///     .build()?;
///
/// // GET request
/// let user: Response<User> = client.get("/users/123").await?;
/// println!("User: {}", user.data.name);
///
/// // POST request
/// let new_user = CreateUser {
///     name: "Alice".to_string(),
///     email: "alice@example.com".to_string(),
/// };
/// let created: Response<User> = client.post("/users", &new_user).await?;
/// println!("Created user with ID: {}", created.data.id);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http_client: reqwest::Client,
    base_url: Url,
    default_headers: HeaderMap,
    retry_strategy: RetryStrategy,
    retry_predicate: Box<dyn RetryPredicate>,
    timeout: Option<Duration>,
    rate_limit_config: RateLimitConfig,
}

impl Client {
    /// Creates a new `ClientBuilder` for configuring a client.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    /// Makes a typed HTTP request.
    ///
    /// This is the main method for making requests. It handles serialization,
    /// retries, logging, and deserialization.
    ///
    /// # Type Parameters
    ///
    /// * `Req` - The request body type (must implement `Serialize`)
    /// * `Res` - The response body type (must implement `DeserializeOwned`)
    ///
    /// # Arguments
    ///
    /// * `metadata` - Request metadata (method, path, headers, etc.)
    /// * `body` - Optional request body
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::{Client, metadata::RequestMetadata};
    /// use http::Method;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize)]
    /// struct Request { query: String }
    ///
    /// #[derive(Deserialize)]
    /// struct ApiResponse { results: Vec<String> }
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let metadata = RequestMetadata::new(Method::POST, "/search");
    /// let request = Request { query: "rust".to_string() };
    ///
    /// let response = client.call::<_, ApiResponse>(metadata, Some(&request)).await?;
    /// println!("Found {} results", response.data.results.len());
    /// # Ok(())
    /// # }
    /// ```
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn call<Req, Res>(
        &self,
        metadata: RequestMetadata,
        body: Option<&Req>,
    ) -> Result<Response<Res>>
    where
        Req: Serialize,
        Res: DeserializeOwned + 'static,
    {
        let client = self.clone();
        self.retry_loop(&metadata, body, move |response, latency, attempt| {
            let client = client.clone();
            Box::pin(async move { client.parse_response(response, latency, attempt).await })
        })
        .await
    }

    /// Makes an HTTP request and returns the response as raw bytes.
    ///
    /// This method is useful when you need to handle non-JSON responses such as:
    /// - Binary files (images, PDFs, etc.)
    /// - Plain text responses
    /// - Custom data formats
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use calleen::metadata::RequestMetadata;
    /// use http::Method;
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// // Download an image
    /// let metadata = RequestMetadata::new(Method::GET, "/images/logo.png");
    /// let response = client.call_bytes(metadata, None::<&()>).await?;
    /// let image_bytes = response.data;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn call_bytes<Req>(
        &self,
        metadata: RequestMetadata,
        body: Option<&Req>,
    ) -> Result<Response<Vec<u8>>>
    where
        Req: Serialize,
    {
        let client = self.clone();
        self.retry_loop(&metadata, body, move |response, latency, attempt| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .parse_bytes_response(response, latency, attempt)
                    .await
            })
        })
        .await
    }

    /// Shared retry loop used by [`Self::call`] and [`Self::call_bytes`].
    ///
    /// Accepts a `parser` closure that converts a raw `reqwest::Response` into
    /// the desired `Response<T>`, allowing the retry/backoff logic to live in
    /// one place.
    async fn retry_loop<Req, T>(
        &self,
        metadata: &RequestMetadata,
        body: Option<&Req>,
        parser: impl Fn(
            reqwest::Response,
            Duration,
            usize,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response<T>>> + Send + 'static>,
        >,
    ) -> Result<Response<T>>
    where
        Req: Serialize,
        T: 'static,
    {
        let start_time = Instant::now();
        let mut attempt = 0;
        let mut last_error = None;

        loop {
            attempt += 1;

            let result = match self.execute_request(metadata, body, attempt).await {
                Ok(response) => {
                    let latency = start_time.elapsed();
                    parser(response, latency, attempt).await
                }
                Err(e) => Err(e),
            };

            match result {
                Ok(response) => return Ok(response),
                Err(e) => {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        error = %e,
                        attempt = attempt,
                        method = %metadata.method,
                        path = %metadata.path,
                        "Request failed"
                    );

                    // Check if we should retry
                    if !self.inner.retry_predicate.should_retry(&e, attempt) {
                        return Err(e);
                    }

                    // Determine retry delay - prefer rate limit info if available
                    // but still respect max_retries
                    let delay = match self.inner.retry_strategy.delay_for_attempt(attempt) {
                        Some(normal_delay) => {
                            // We have retries remaining - check if rate limit delay should override
                            if self.inner.rate_limit_config.enabled {
                                if let Some(rate_limit_delay) =
                                    e.rate_limit_delay(self.inner.rate_limit_config.max_wait)
                                {
                                    #[cfg(feature = "tracing")]
                                    tracing::info!(
                                        rate_limit_delay_ms = rate_limit_delay.as_millis(),
                                        attempt = attempt,
                                        max_wait_secs =
                                            self.inner.rate_limit_config.max_wait.as_secs(),
                                        "Rate limited - waiting before retry"
                                    );
                                    Some(rate_limit_delay)
                                } else {
                                    Some(normal_delay)
                                }
                            } else {
                                Some(normal_delay)
                            }
                        }
                        None => None, // No retries remaining
                    };

                    // Check if we have more retries available
                    if let Some(delay) = delay {
                        #[cfg(feature = "tracing")]
                        if e.rate_limit_info().is_none() {
                            tracing::info!(
                                delay_ms = delay.as_millis(),
                                attempt = attempt,
                                "Retrying request after delay"
                            );
                        }

                        tokio::time::sleep(delay).await;
                        last_error = Some(e);
                    } else {
                        // No more retries
                        return Err(Error::MaxRetriesExceeded {
                            attempts: attempt,
                            last_error: Box::new(last_error.unwrap_or(e)),
                        });
                    }
                }
            }
        }
    }

    /// Executes a single request attempt.
    async fn execute_request<Req>(
        &self,
        metadata: &RequestMetadata,
        body: Option<&Req>,
        _attempt: usize,
    ) -> Result<reqwest::Response>
    where
        Req: Serialize,
    {
        // Build the full URL
        let mut url = self.inner.base_url.clone();
        url.set_path(&metadata.path);

        // Add query parameters
        for (key, value) in &metadata.query_params {
            url.query_pairs_mut().append_pair(key, value);
        }

        #[cfg(feature = "tracing")]
        tracing::debug!(
            method = %metadata.method,
            url = %url,
            attempt = _attempt,
            "Executing HTTP request"
        );

        // Build the request
        let mut request = self.inner.http_client.request(metadata.method.clone(), url);

        // Add default headers
        for (name, value) in &self.inner.default_headers {
            request = request.header(name, value);
        }

        // Add request-specific headers
        for (name, value) in &metadata.headers {
            request = request.header(name, value);
        }

        // Add timeout if configured
        if let Some(timeout) = self.inner.timeout {
            request = request.timeout(timeout);
        }

        // Add body if provided (JSON takes precedence over form data)
        if let Some(body) = body {
            let json = serde_json::to_value(body)
                .map_err(|e| Error::SerializationFailed(e.to_string()))?;
            request = request.json(&json);
        } else if let Some(form_data) = &metadata.form_data {
            request = request.form(form_data);
        }

        // Execute the request
        let response = request.send().await?;

        Ok(response)
    }

    /// Parses the response and returns a typed `Response`.
    async fn parse_response<Res>(
        &self,
        response: reqwest::Response,
        latency: Duration,
        attempts: usize,
    ) -> Result<Response<Res>>
    where
        Res: DeserializeOwned,
    {
        let status = response.status();
        let headers = response.headers().clone();

        #[cfg(feature = "tracing")]
        tracing::info!(
            status = status.as_u16(),
            latency_ms = latency.as_millis(),
            attempts = attempts,
            "Received HTTP response"
        );

        // Check for HTTP errors (non-2xx)
        if !status.is_success() {
            let raw_response = response.text().await.unwrap_or_default();

            // Parse rate limit info if enabled
            let rate_limit_info = if self.inner.rate_limit_config.enabled {
                let info = crate::rate_limit::RateLimitInfo::from_headers(&headers);
                if info.is_rate_limited() {
                    Some(info)
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(feature = "tracing")]
            if status.is_client_error() {
                tracing::error!(
                    status = status.as_u16(),
                    response = %raw_response,
                    "Client error (4xx)"
                );
            } else if status.is_server_error() {
                tracing::warn!(
                    status = status.as_u16(),
                    response = %raw_response,
                    "Server error (5xx)"
                );
            }

            return Err(Error::HttpError {
                status,
                raw_response: raw_response.into_boxed_str(),
                headers: Box::new(headers),
                rate_limit_info,
            });
        }

        // Get raw response text
        let raw_body = response.text().await?;

        // A successful response with an empty body gets replaced with "{}"
        let parse_target = if raw_body.is_empty() && status.is_success() {
            "{}"
        } else {
            &raw_body
        };

        // Try to deserialize
        match serde_json::from_str::<Res>(parse_target) {
            Ok(data) => Ok(Response::new(
                data, raw_body, status, headers, latency, attempts,
            )),
            Err(e) => {
                #[cfg(feature = "tracing")]
                tracing::error!(
                    error = %e,
                    raw_response = %raw_body,
                    "Failed to deserialize response"
                );

                Err(Error::DeserializationFailed {
                    raw_response: raw_body,
                    serde_error: e.to_string(),
                    status,
                })
            }
        }
    }

    /// Parses the response as raw bytes and returns a `Response<Vec<u8>>`.
    async fn parse_bytes_response(
        &self,
        response: reqwest::Response,
        latency: Duration,
        attempts: usize,
    ) -> Result<Response<Vec<u8>>> {
        let status = response.status();
        let headers = response.headers().clone();

        #[cfg(feature = "tracing")]
        tracing::info!(
            status = status.as_u16(),
            latency_ms = latency.as_millis(),
            attempts = attempts,
            "Received HTTP response (bytes)"
        );

        // Check for HTTP errors (non-2xx)
        if !status.is_success() {
            let raw_response = response.text().await.unwrap_or_default();

            // Parse rate limit info if enabled
            let rate_limit_info = if self.inner.rate_limit_config.enabled {
                let info = crate::rate_limit::RateLimitInfo::from_headers(&headers);
                if info.is_rate_limited() {
                    Some(info)
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(feature = "tracing")]
            if status.is_client_error() {
                tracing::error!(
                    status = status.as_u16(),
                    response = %raw_response,
                    "Client error (4xx)"
                );
            } else if status.is_server_error() {
                tracing::warn!(
                    status = status.as_u16(),
                    response = %raw_response,
                    "Server error (5xx)"
                );
            }

            return Err(Error::HttpError {
                status,
                raw_response: raw_response.into_boxed_str(),
                headers: Box::new(headers),
                rate_limit_info,
            });
        }

        // Get raw response bytes
        let bytes = response.bytes().await?;
        let data = bytes.to_vec();

        // For the raw_body field in Response, we'll store a string representation
        // (either UTF-8 if valid, or a placeholder indicating binary data and its length)
        let raw_body = match std::str::from_utf8(&data) {
            Ok(text) => text.to_owned(),
            Err(_) => format!("<binary data: {} bytes>", data.len()),
        };

        Ok(Response::new(
            data, raw_body, status, headers, latency, attempts,
        ))
    }

    /// Makes a GET request to the specified path.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct User { name: String }
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let user: calleen::Response<User> = client.get("/users/123").await?;
    /// println!("User: {}", user.data.name);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn get<Res>(&self, path: impl Into<String>) -> Result<Response<Res>>
    where
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::GET, path);
        self.call::<(), Res>(metadata, None).await
    }

    /// Makes a POST request to the specified path with a JSON body.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize)]
    /// struct CreateUser { name: String }
    ///
    /// #[derive(Deserialize)]
    /// struct User { id: u64, name: String }
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let request = CreateUser { name: "Alice".to_string() };
    /// let user: calleen::Response<User> = client.post("/users", &request).await?;
    /// println!("Created user ID: {}", user.data.id);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn post<Req, Res>(&self, path: impl Into<String>, body: &Req) -> Result<Response<Res>>
    where
        Req: Serialize,
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::POST, path);
        self.call(metadata, Some(body)).await
    }

    /// Makes a POST request with form-encoded data.
    ///
    /// The data is sent as `application/x-www-form-urlencoded`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn post_form<Res>(
        &self,
        path: impl Into<String>,
        form_data: HashMap<String, String>,
    ) -> Result<Response<Res>>
    where
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::POST, path).with_form_data(form_data);
        self.call::<(), Res>(metadata, None).await
    }

    /// Makes a PUT request to the specified path with a JSON body.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn put<Req, Res>(&self, path: impl Into<String>, body: &Req) -> Result<Response<Res>>
    where
        Req: Serialize,
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::PUT, path);
        self.call(metadata, Some(body)).await
    }

    /// Makes a DELETE request to the specified path.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn delete<Res>(&self, path: impl Into<String>) -> Result<Response<Res>>
    where
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::DELETE, path);
        self.call::<(), Res>(metadata, None).await
    }

    /// Makes a PATCH request to the specified path with a JSON body.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn patch<Req, Res>(
        &self,
        path: impl Into<String>,
        body: &Req,
    ) -> Result<Response<Res>>
    where
        Req: Serialize,
        Res: DeserializeOwned + 'static,
    {
        let metadata = RequestMetadata::new(Method::PATCH, path);
        self.call(metadata, Some(body)).await
    }

    /// Makes a GET request and returns the response as raw bytes.
    ///
    /// This is useful for downloading binary files, images, or any non-JSON content.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// // Download an image
    /// let response = client.get_bytes("/images/logo.png").await?;
    /// let image_data = response.data;
    /// std::fs::write("logo.png", image_data)?;
    ///
    /// // Download a PDF
    /// let pdf_response = client.get_bytes("/documents/report.pdf").await?;
    /// println!("Downloaded {} bytes", pdf_response.data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_bytes(&self, path: impl Into<String>) -> Result<Response<Vec<u8>>> {
        let metadata = RequestMetadata::new(Method::GET, path);
        self.call_bytes::<()>(metadata, None).await
    }

    /// Makes a POST request with a JSON body and returns the response as raw bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct GenerateRequest {
    ///     format: String,
    ///     data: String,
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let request = GenerateRequest {
    ///     format: "pdf".to_string(),
    ///     data: "Hello World".to_string(),
    /// };
    ///
    /// // Generate a PDF and get raw bytes
    /// let response = client.post_bytes("/generate", &request).await?;
    /// std::fs::write("output.pdf", response.data)?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn post_bytes<Req>(
        &self,
        path: impl Into<String>,
        body: &Req,
    ) -> Result<Response<Vec<u8>>>
    where
        Req: Serialize,
    {
        let metadata = RequestMetadata::new(Method::POST, path);
        self.call_bytes(metadata, Some(body)).await
    }

    /// Makes a PUT request with a JSON body and returns the response as raw bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct UpdateData { content: Vec<u8> }
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let data = UpdateData { content: vec![0x01, 0x02, 0x03] };
    /// let response = client.put_bytes("/files/123", &data).await?;
    /// println!("Updated file, server returned {} bytes", response.data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn put_bytes<Req>(
        &self,
        path: impl Into<String>,
        body: &Req,
    ) -> Result<Response<Vec<u8>>>
    where
        Req: Serialize,
    {
        let metadata = RequestMetadata::new(Method::PUT, path);
        self.call_bytes(metadata, Some(body)).await
    }

    /// Makes a DELETE request and returns the response as raw bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let response = client.delete_bytes("/files/123").await?;
    /// // Some APIs might return the deleted file content
    /// if !response.data.is_empty() {
    ///     println!("Deleted file was {} bytes", response.data.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_bytes(&self, path: impl Into<String>) -> Result<Response<Vec<u8>>> {
        let metadata = RequestMetadata::new(Method::DELETE, path);
        self.call_bytes::<()>(metadata, None).await
    }

    /// Makes a PATCH request with a JSON body and returns the response as raw bytes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::Client;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct PatchData { partial_update: String }
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .build()?;
    ///
    /// let patch = PatchData { partial_update: "new value".to_string() };
    /// let response = client.patch_bytes("/resources/123", &patch).await?;
    /// println!("Patched resource, got {} bytes back", response.data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn patch_bytes<Req>(
        &self,
        path: impl Into<String>,
        body: &Req,
    ) -> Result<Response<Vec<u8>>>
    where
        Req: Serialize,
    {
        let metadata = RequestMetadata::new(Method::PATCH, path);
        self.call_bytes(metadata, Some(body)).await
    }
}

/// Builder for configuring and creating a [`Client`].
///
/// # Examples
///
/// ```no_run
/// use calleen::{ClientBuilder, RetryStrategy};
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), calleen::Error> {
/// let client = ClientBuilder::new()
///     .base_url("https://api.example.com")?
///     .timeout(Duration::from_secs(30))
///     .retry_strategy(RetryStrategy::ExponentialBackoff {
///         initial_delay: Duration::from_millis(100),
///         max_delay: Duration::from_secs(10),
///         max_retries: 3,
///         jitter: true,
///     })
///     .default_header("User-Agent", "my-app/1.0")?
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct ClientBuilder {
    base_url: Option<Url>,
    default_headers: HeaderMap,
    retry_strategy: RetryStrategy,
    retry_predicate: Option<Box<dyn RetryPredicate>>,
    timeout: Option<Duration>,
    rate_limit_config: RateLimitConfig,
}

impl ClientBuilder {
    /// Creates a new `ClientBuilder` with default settings.
    pub fn new() -> Self {
        Self {
            base_url: None,
            default_headers: HeaderMap::new(),
            retry_strategy: RetryStrategy::None,
            retry_predicate: None,
            timeout: None,
            rate_limit_config: RateLimitConfig::default(),
        }
    }

    /// Sets the base URL for all requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid.
    pub fn base_url(mut self, url: impl AsRef<str>) -> Result<Self> {
        self.base_url = Some(Url::parse(url.as_ref())?);
        Ok(self)
    }

    /// Adds a default header that will be included in all requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the header name or value is invalid.
    pub fn default_header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Result<Self> {
        let name = HeaderName::try_from(name.as_ref())
            .map_err(|e| Error::ConfigurationError(format!("Invalid header name: {}", e)))?;
        let value = HeaderValue::try_from(value.as_ref())
            .map_err(|e| Error::ConfigurationError(format!("Invalid header value: {}", e)))?;
        self.default_headers.insert(name, value);
        Ok(self)
    }

    /// Sets HTTP Basic Authentication credentials for all requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the authorization header value is invalid.
    pub fn basic_auth(self, username: impl AsRef<str>, password: impl AsRef<str>) -> Result<Self> {
        use base64::Engine;
        let credentials = format!("{}:{}", username.as_ref(), password.as_ref());
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        self.default_header("Authorization", format!("Basic {}", encoded))
    }

    /// Sets the retry strategy for failed requests.
    pub fn retry_strategy(mut self, strategy: RetryStrategy) -> Self {
        self.retry_strategy = strategy;
        self
    }

    /// Sets a custom retry predicate.
    ///
    /// By default, requests are retried based on `Error::is_retryable()`.
    pub fn retry_predicate(mut self, predicate: Box<dyn RetryPredicate>) -> Self {
        self.retry_predicate = Some(predicate);
        self
    }

    /// Sets the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the rate limit configuration.
    ///
    /// By default, rate limit handling is enabled with sensible defaults.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use calleen::{Client, rate_limit::RateLimitConfig};
    /// use std::time::Duration;
    ///
    /// # async fn example() -> Result<(), calleen::Error> {
    /// let client = Client::builder()
    ///     .base_url("https://api.example.com")?
    ///     .rate_limit_config(RateLimitConfig::builder()
    ///         .max_wait(Duration::from_secs(60))
    ///         .build())
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.rate_limit_config = config;
        self
    }

    /// Builds the configured `Client`.
    ///
    /// # Errors
    ///
    /// Returns an error if no base URL was provided or if the client
    /// configuration is invalid.
    pub fn build(self) -> Result<Client> {
        let base_url = self
            .base_url
            .ok_or_else(|| Error::ConfigurationError("Base URL is required".to_string()))?;

        let http_client = reqwest::Client::builder().build().map_err(|e| {
            Error::ConfigurationError(format!("Failed to build HTTP client: {}", e))
        })?;

        let retry_predicate = self
            .retry_predicate
            .unwrap_or_else(|| Box::new(RetryOnRetryable));

        Ok(Client {
            inner: Arc::new(ClientInner {
                http_client,
                base_url,
                default_headers: self.default_headers,
                retry_strategy: self.retry_strategy,
                retry_predicate,
                timeout: self.timeout,
                rate_limit_config: self.rate_limit_config,
            }),
        })
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
