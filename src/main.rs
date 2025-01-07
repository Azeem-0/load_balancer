mod algorithms;
mod handlers;

use std::{
    fs,
    sync::{Arc, Mutex},
};

use algorithms::round_robin::{Config, RoundRobin};
use axum::{routing::any, Router};
use handlers::load_balancer::load_balancer;

#[tokio::main]
async fn main() {
    let config_content = fs::read_to_string("Config.toml").expect("Failed to read Config.toml");

    let config: Config = toml::from_str(&config_content).expect("Failed to parse Config.toml");

    let rpc_servers = config.rpc_urls.servers;

    let round_robin = Arc::new(Mutex::new(RoundRobin::new(rpc_servers)));

    let round_robin_clone;

    {
        let round_robin = round_robin.lock().unwrap();

        round_robin_clone = round_robin.clone();
    }

    tokio::spawn(async move {
        round_robin_clone.refill_limits().await;
    });

    let app = Router::new()
        .route("/{*path}", any(load_balancer))
        .with_state(round_robin);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
