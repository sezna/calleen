use serde::{Deserialize, Serialize};

struct Metadata {
use_tls: bool,
url: Arc<str>,
retry_strategy: RetryStrategy,
}

enum RetryStrategy {
    None,
    ExponentialBackoff {
    /// duration in milliseconds
        ms: usize,
        exponent: usize,
    },
    Linear
 {
    ms: usize
}    
    
}

// TODO: use thiserror and enumerate error possibilities
pub enum Error {}

pub async fn call<Req, Res>(meta: Metadata, req: Req) -> Result<Res, Error>
where
    Req: Serialize<'_>,
    Res: Deserialize<'_>,
{
    // serialize the request
    // set up the reqwest client according to metadata
    // send request
    // log: response latency, status code, etc
    // error log if status code is 4xx
    // warn log if status code is 5xx
    // take raw response text out
    // try to deserialize into Res
    // if that fails, error log the original text that failed to deserialize as well as the serde err msg
    // 
}
