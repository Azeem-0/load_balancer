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
use reqwest::{Method, RequestBuilder, Response as ReqwestResponse, StatusCode};

pub async fn load_balancer(
    Path(chain): Path<String>,
    State(state): State<Arc<LoadBalancer>>,
    request: axum::http::Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let round_robin = {
        let load_balancers = state.load_balancers.lock().unwrap();
        let rr = load_balancers.get(&chain);
        if let None = rr {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(format!("Invalid chain: {}", chain)))
                .unwrap());
        }
        rr.unwrap().clone()
    };

    let max_size = 1024 * 1024;

    let method = request.method().clone();

    let body_bytes = body::to_bytes(request.into_body(), max_size)
        .await
        .unwrap_or_default();

    let forwarded_request = retry_with_backoff(
        3,
        || {
            Box::pin(get_forward_request(
                round_robin.clone(),
                method.clone(),
                body_bytes.clone(),
            ))
        },
        round_robin.clone(),
    )
    .await;

    match forwarded_request {
        Some(response) => {
            let status = response.status();
            let body_bytes = response.bytes().await.unwrap_or_default();
            let forwarded_response = Response::builder()
                .status(status)
                .header("Content-Type", "application/json")
                .body(Body::from(body_bytes))
                .unwrap();
            return Ok(forwarded_response);
        }
        None => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(Body::from("Bad Gateway"))
                .unwrap());
        }
    }
}

async fn get_forward_request(
    state: Arc<Mutex<RoundRobin>>,
    method: Method,
    body_bytes: Bytes,
) -> Option<RequestBuilder> {
    let uri;

    {
        let round_robin = state.lock().unwrap();
        uri = round_robin.get_next();
    }

    println!("{:?}", uri);

    if let Some(uri) = uri {
        println!("Forwarding request to : {}", &uri);

        let client = reqwest::Client::new();

        let mut forwarded_request = client.request(method, &uri);

        forwarded_request = forwarded_request.header("Content-Type", "application/json");
        forwarded_request = forwarded_request.body(body_bytes);

        return Some(forwarded_request);
    } else {
        None
    }
}

async fn retry_with_backoff<F>(
    max_retries: u32,
    mut task: F,
    state: Arc<Mutex<RoundRobin>>,
) -> Option<ReqwestResponse>
where
    F: FnMut() -> Pin<Box<dyn Future<Output = Option<RequestBuilder>> + Send>>, // Closure returns a Future
{
    let mut retries = 0;
    let mut delay = Duration::from_millis(100);

    while retries < max_retries {
        let result = task().await;

        if let Some(result) = result {
            if let Ok(res) = result.send().await {
                if res.status() == 200 {
                    return Some(res);
                }
            }
        }

        println!("Retrying with another RPC Url.");

        retries += 1;
        tokio::time::sleep(delay).await;
        delay *= 100; // Exponential backoff

        {
            let round_robin = state.lock().unwrap();
            round_robin.retry_connection();
        }
    }

    return None;
}
