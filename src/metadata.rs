//! Request metadata and configuration types.

use http::{HeaderMap, HeaderName, HeaderValue, Method};
use std::collections::HashMap;

/// Metadata for an individual HTTP request.
///
/// This type contains all the configuration needed to make a single HTTP request,
/// including headers, query parameters, method, and path.
#[derive(Debug, Clone)]
pub struct RequestMetadata {
    /// The HTTP method (GET, POST, etc.).
    pub method: Method,

    /// The request path (relative to the base URL).
    pub path: String,

    /// Additional headers for this request.
    pub headers: HeaderMap,

    /// Query parameters for this request.
    pub query_params: HashMap<String, String>,
}

impl RequestMetadata {
    /// Creates a new `RequestMetadata` with the given method and path.
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            headers: HeaderMap::new(),
            query_params: HashMap::new(),
        }
    }

    /// Adds a header to the request.
    ///
    /// # Errors
    ///
    /// Returns an error if the header name or value is invalid.
    pub fn with_header(
        mut self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Result<Self, crate::Error> {
        let name = HeaderName::try_from(name.as_ref())
            .map_err(|e| crate::Error::ConfigurationError(format!("Invalid header name: {}", e)))?;
        let value = HeaderValue::try_from(value.as_ref())
            .map_err(|e| crate::Error::ConfigurationError(format!("Invalid header value: {}", e)))?;
        self.headers.insert(name, value);
        Ok(self)
    }

    /// Adds a query parameter to the request.
    pub fn with_query_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query_params.insert(key.into(), value.into());
        self
    }

    /// Adds multiple query parameters to the request.
    pub fn with_query_params(
        mut self,
        params: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        self.query_params.extend(params);
        self
    }
}

impl Default for RequestMetadata {
    fn default() -> Self {
        Self::new(Method::GET, "")
    }
}
