//! Basic example demonstrating simple GET and POST requests.
//!
//! This example shows how to:
//! - Create a client with basic configuration
//! - Make GET requests to fetch data
//! - Make POST requests to create data
//! - Access response data and metadata
//!
//! Run with: `cargo run --example basic_call`

use calleen::{Client, Error};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Post {
    #[serde(rename = "userId")]
    user_id: u32,
    id: u32,
    title: String,
    body: String,
}

#[derive(Debug, Serialize)]
struct NewPost {
    title: String,
    body: String,
    #[serde(rename = "userId")]
    user_id: u32,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_env_filter("calleen=debug,basic_call=info")
        .init();

    // Create a client for the JSONPlaceholder API
    let client = Client::builder()
        .base_url("https://jsonplaceholder.typicode.com")?
        .build()?;

    println!("=== GET Request Example ===");
    // Make a GET request to fetch a post
    let response = client.get::<Post>("/posts/1").await?;

    println!("Post ID: {}", response.data.id);
    println!("Title: {}", response.data.title);
    println!("Body: {}", response.data.body);
    println!("Request latency: {:?}", response.latency);
    println!("Status code: {}", response.status);
    println!();

    println!("=== POST Request Example ===");
    // Make a POST request to create a new post
    let new_post = NewPost {
        title: "My New Post".to_string(),
        body: "This is the content of my new post!".to_string(),
        user_id: 1,
    };

    let response = client.post::<_, Post>("/posts", &new_post).await?;

    println!("Created post ID: {}", response.data.id);
    println!("Title: {}", response.data.title);
    println!("Request latency: {:?}", response.latency);
    println!();

    println!("=== Accessing Response Metadata ===");
    println!("Raw response length: {} bytes", response.raw_body.len());
    println!("Content-Type: {:?}", response.header("content-type"));
    println!("Was retried: {}", response.was_retried());

    Ok(())
}
