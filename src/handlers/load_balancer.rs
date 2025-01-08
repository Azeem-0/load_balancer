use std::{
    convert::Infallible,
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
    State(state): State<LoadBalancer>,
    request: axum::http::Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let round_robin = {
        let rr = state.load_balancers.get(&chain);
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

    let forwarded_request =
        retry_with_backoff(3, round_robin.clone(), method.clone(), body_bytes.clone()).await;

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

async fn retry_with_backoff(
    max_retries: u32,
    state: Arc<Mutex<RoundRobin>>,
    method: Method,
    body_bytes: Bytes,
) -> Option<ReqwestResponse> {
    let mut retries = 0;
    let delay = Duration::from_millis(1);

    while retries < max_retries {
        let result = get_forward_request(state.clone(), method.clone(), body_bytes.clone()).await;
        println!("Retry number : {}", retries);

        if let Some(result) = result {
            if let Ok(res) = result.send().await {
                if res.status() == 200 {
                    return Some(res);
                }
            }
        }

        println!("Retrying with another RPC Url.");

        {
            let round_robin = state.lock().unwrap();
            round_robin.retry_connection();
        }

        retries += 1;
        tokio::time::sleep(delay).await;
    }

    return None;
}

async fn get_forward_request(
    state: Arc<Mutex<RoundRobin>>,
    method: Method,
    body_bytes: Bytes,
) -> Option<RequestBuilder> {
    let uri;

    {
        let mut round_robin = state.lock().unwrap();
        uri = round_robin.get_next();
    }

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::algorithms::round_robin::{RoundRobin, RpcServer};
    use axum::http::Request;

    use tokio::test;
    fn create_test_servers() -> Vec<RpcServer> {
        vec![
            RpcServer {
                url: "https://sepolia.drpc.org/".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://polygon-rpc.com".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ]
    }

    // Helper function to create a test request
    fn create_test_request() -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("https://sepolia.drpc.org/")
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}"#,
            ))
            .unwrap()
    }

    #[test]
    async fn test_successful_request_forwarding() {
        let servers = create_test_servers();
        let mock_round_robin = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("sepolia".to_string(), mock_round_robin);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };

        let request = create_test_request();

        let path: Path<String> = Path("sepolia".to_string());
        let response = load_balancer(path, State(lbs), request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    async fn test_request_headers_forwarded() {
        let servers = create_test_servers();
        let mock_round_robin = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("sepolia".to_string(), mock_round_robin);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };

        let request = Request::builder()
            .method("POST")
            .uri("http://test.com")
            .header("X-Custom-Header", "test-value")
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}"#,
            ))
            .unwrap();

        // TODO: Add assertions for header forwarding once HTTP mocking is implemented
        let path: Path<String> = Path("sepolia".to_string());

        let response = load_balancer(path, State(lbs), request).await.unwrap();

        assert_eq!(response.headers()["Content-Type"], "application/json");
    }

    #[test]
    async fn test_retry_on_failure() {
        println!("entered retry testing");
        let request = create_test_request();
        let servers = vec![
            RpcServer {
                url: "https://sepolia.d.org".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://endpoints.omniatech.io/v1/eth/sepolia/public".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://sepolia.drpc.org".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let mock_round_robin = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("sepolia".to_string(), mock_round_robin);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };
        let path: Path<String> = Path("sepolia".to_string());
        println!("before resp");
        let response = load_balancer(path, State(lbs), request).await.unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    async fn test_multiple_chains() {
        let request = create_test_request();
        let servers = vec![
            RpcServer {
                url: "https://sepolia.d.org".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://sepolia.drpc.org".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://endpoints.omniatech.io/v1/eth/sepolia/public".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let polygon = vec![
            RpcServer {
                url: "https://polygon-rpc.com".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://another-polygon-rpc.com".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let fantom = vec![
            RpcServer {
                url: "https://rpc.ftm.tools".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://another-fantom-rpc.com".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let sepolia_servers = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let polygon_servers = Arc::new(Mutex::new(RoundRobin::new(polygon)));
        let fantom_servers = Arc::new(Mutex::new(RoundRobin::new(fantom)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("sepolia".to_string(), sepolia_servers);
        chains.insert("polygon".to_string(), polygon_servers);
        chains.insert("fantom".to_string(), fantom_servers);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };

        {
            let round_robin_lb = &lbs.load_balancers;

            for round_robin in round_robin_lb.values() {
                let mut rr_clone;
                {
                    let rr = round_robin.lock().unwrap();
                    rr_clone = rr.clone();
                }

                tokio::spawn(async move {
                    rr_clone.refill_limits().await;
                });
            }
        }

        let path: Path<String> = Path("sepolia".to_string());
        let response = load_balancer(path, State(lbs), request).await.unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);

        // let req2 = create_test_request();
        // let path: Path<String> = Path("polygon".to_string());
        // let response = load_balancer(path, State(lbs.clone()), req2).await.unwrap();
        // println!("{}", response.status());
        // assert_eq!(response.status(), StatusCode::OK);

        // let req3 = create_test_request();
        // let path: Path<String> = Path("fantom".to_string());
        // let response = load_balancer(path, State(lbs), req3).await.unwrap();
        // println!("{}", response.status());
        // assert_eq!(response.status(), StatusCode::OK);
    }
}
