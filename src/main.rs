mod algorithms;
mod handlers;
mod models;

use std::{
    fs,
    sync::{Arc, Mutex},
};

use algorithms::round_robin::RoundRobin;
use axum::{
    routing::{any, get},
    Extension, Router,
};
use handlers::load_balancer::load_balancer;
use models::rpc_model::Config;

#[tokio::main]
async fn main() {
    let config_content = fs::read_to_string("Config.toml").expect("Failed to read Config.toml");

    let config: Config = toml::from_str(&config_content).expect("Failed to parse Config.toml");

    let rpc_servers = config.rpc_urls.server;

    let round_robin = Arc::new(Mutex::new(RoundRobin::new(rpc_servers)));

    let app = Router::new()
        .route("/{*path}", any(load_balancer))
        .layer(Extension(round_robin));
    // .with_state(round_robin);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn root() -> &'static str {
    "Hello, World!"
}
