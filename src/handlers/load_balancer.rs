use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::{
    body::{self, Body, Bytes},
    extract::{Request, State},
    response::Response,
};
use reqwest::{Method, RequestBuilder, StatusCode};

use crate::algorithms::round_robin::RoundRobin;

pub async fn load_balancer(
    State(state): State<Arc<Mutex<RoundRobin>>>,
    request: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let retry_count = 3;
    let attempts = 0;

    let max_size = 1024 * 1024;

    let method = request.method().clone();

    let body_bytes = body::to_bytes(request.into_body(), max_size)
        .await
        .unwrap_or_default();

    while attempts < retry_count {
        let forwarded_request =
            get_forward_request(&state, method.clone(), body_bytes.clone()).await;

        if let None = forwarded_request {
            return Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Content-Type", "application/json")
                .body(Body::from("No available RPC URLs"))
                .unwrap());
        }
        match forwarded_request.unwrap().send().await {
            Ok(response) => {
                let status = response.status();
                let body_bytes = response.bytes().await.unwrap_or_default();
                let forwarded_response = Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json")
                    .body(Body::from(body_bytes))
                    .unwrap();
                return Ok(forwarded_response);
            }
            Err(_) => {
                // here we have to write the retry logic...
                let url;
                {
                    let round_robin = state.lock().unwrap();
                    url = round_robin.retry_connection();
                }

                if let None = url {
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .header("Content-Type", "application/json")
                        .body(Body::from("Bad Gateway"))
                        .unwrap());
                }
            }
        }
    }

    // If all retries fail, return a Bad Gateway response
    Ok(Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("Content-Type", "application/json")
        .body(Body::from("Bad Gateway"))
        .unwrap())
}

async fn get_forward_request(
    state: &Arc<Mutex<RoundRobin>>,
    request: Method,
    body_bytes: Bytes,
) -> Option<RequestBuilder> {
    let uri;

    {
        let round_robin = state.lock().unwrap();
        uri = round_robin.get_next();
    }

    if let Some(uri) = uri {
        println!("Forwarding request to : {}", &uri);

        let client = reqwest::Client::new();

        let mut forwarded_request = client.request(request, &uri);

        forwarded_request = forwarded_request.header("Content-Type", "application/json");
        forwarded_request = forwarded_request.body(body_bytes);

        return Some(forwarded_request);
    } else {
        None
    }
}
