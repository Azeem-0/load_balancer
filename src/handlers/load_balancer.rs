use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::algorithms::round_robin::{LoadBalancer, RoundRobin};
use axum::{
    body::{self, Body, Bytes},
    extract::{Path, State},
    response::Response,
};
use reqwest::{Method, Response as ReqwestResponse, StatusCode};

pub async fn load_balancer(
    Path(chain): Path<String>,
    State(state): State<Arc<Mutex<LoadBalancer>>>,
    request: axum::http::Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let max_size = 1024 * 1024;

    let method = request.method().clone();
    let body_bytes = body::to_bytes(request.into_body(), max_size)
        .await
        .unwrap_or_default();

    let round_robin = {
        let load_balancer = state.lock().unwrap();
        load_balancer.load_balancers.get(&chain).cloned()
    };

    if let Some(round_robin) = round_robin {
        let forwarded_response = retry_with_backoff(3, || {
            Box::pin(get_forward_request(
                round_robin.clone(),
                method.clone(),
                body_bytes.clone(),
            ))
        })
        .await;

        match forwarded_response {
            Some(response) => {
                let status = response.status();
                let body_bytes = response.bytes().await.unwrap_or_default();
                return Ok(Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body_bytes))
                    .unwrap());
            }
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .header("Content-Type", "application/json")
                    .body(Body::from("No available RPC URLs"))
                    .unwrap());
            }
        }
    }

    // If no RoundRobin instance is found for the chain
    Ok(Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header("Content-Type", "application/json")
        .body(Body::from(format!("Invalid chain: {}", chain)))
        .unwrap())
}

async fn get_forward_request(
    round_robin: RoundRobin,
    method: Method,
    body_bytes: Bytes,
) -> Option<reqwest::RequestBuilder> {
    if let Some(uri) = round_robin.get_next() {
        println!("Forwarding request to: {}", &uri);

        let client = reqwest::Client::new();
        let mut forwarded_request = client.request(method, &uri);
        forwarded_request = forwarded_request.header("Content-Type", "application/json");
        forwarded_request = forwarded_request.body(body_bytes);

        return Some(forwarded_request);
    }

    None
}

async fn retry_with_backoff<F>(max_retries: u32, mut task: F) -> Option<ReqwestResponse>
where
    F: FnMut() -> Pin<Box<dyn Future<Output = Option<reqwest::RequestBuilder>> + Send>>, // Closure returns a Future
{
    let mut retries = 0;
    let mut delay = Duration::from_secs(1);

    while retries < max_retries {
        if let Some(request_builder) = task().await {
            match request_builder.send().await {
                Ok(response) => return Some(response),
                Err(err) => {
                    println!("Request failed: {}. Retrying...", err);
                }
            }
        }

        retries += 1;
        tokio::time::sleep(delay).await;
        delay *= 2; // Exponential backoff
    }

    println!("All retries failed. Returning None.");
    None
}
