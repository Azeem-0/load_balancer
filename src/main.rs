mod algorithms;
mod handlers;

use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use algorithms::round_robin::{Config, LoadBalancer, RoundRobin};
use axum::{routing::any, Router};
use handlers::load_balancer::load_balancer;

pub async fn initialize_load_balancer(config: Config) -> LoadBalancer {
    let mut lb_map = HashMap::new();
    for (chain_name, chain_data) in config.chains {
        let round_robin = Arc::new(Mutex::new(RoundRobin::new(chain_data.rpc_urls)));
        lb_map.insert(chain_name, round_robin);
    }

    Arc::new(LoadBalancer {
        load_balancers: Arc::new(lb_map),
    })
}

#[tokio::main]
async fn main() {
    let config: Config = {
        let config_content = fs::read_to_string("Config.toml").expect("Failed to read Config.toml");
        toml::from_str(&config_content).expect("Failed to parse Config.toml")
    };

    let lb = initialize_load_balancer(config).await;

    for round_robin in lb.load_balancers.values() {
        let rr_clone;

        {
            let rr = round_robin.lock().unwrap();
            rr_clone = rr.clone();
        }

        tokio::spawn(async move {
            rr_clone.refill_limits(Duration::from_secs(10)).await;
        });
    }

    let app = Router::new()
        .route("/{*path}", any(load_balancer))
        .with_state(lb);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
