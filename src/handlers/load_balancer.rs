use std::{
    convert::Infallible,
    fs,
    sync::{Arc, Mutex},
};

use axum::{
    body::{self, Body},
    extract::{Request, State},
    response::Response,
    Extension,
};
use reqwest::StatusCode;

use crate::{algorithms::round_robin::RoundRobin, models::rpc_model::Config};

pub async fn load_balancer(
    State(state): State<Arc<Mutex<RoundRobin>>>,
    request: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let uri;

    {
        let round_robin = state.lock().unwrap();
        uri = round_robin.get_next();
    }

    println!("Forwarding request to : {}", &uri);

    let client = reqwest::Client::new();

    let mut forwarded_request = client.request(request.method().clone(), &uri);

    let max_size = 1024 * 1024;
    let body_bytes = body::to_bytes(request.into_body(), max_size)
        .await
        .unwrap_or_default();

    forwarded_request = forwarded_request.header("Content-Type", "application/json");
    forwarded_request = forwarded_request.body(body_bytes);

    match forwarded_request.send().await {
        Ok(response) => {
            let status = response.status();
            let body_bytes = response.bytes().await.unwrap_or_default();
            let forwarded_response = Response::builder()
                .status(status)
                .header("Content-Type", "application/json")
                .body(Body::from(body_bytes))
                .unwrap();
            Ok(forwarded_response)
        }
        Err(_) => Ok(Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("Content-Type", "application/json")
            .body(Body::from("Bad Gateway"))
            .unwrap()),
    }
}
