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
    State(state): State<Arc<LoadBalancer>>,
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

    let method = Arc::new(request.method().clone());

    let body_bytes = {
        let body = request.into_body();
        let body_bytes = body::to_bytes(body, max_size).await;
        if let Err(_) = body_bytes {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from("Failed to read request body"))
                .unwrap());
        }
        Arc::new(body_bytes.unwrap_or_default())
    };

    let forwarded_request = retry_with_backoff(method, body_bytes, round_robin).await;

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
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Content-Type", "application/json")
                .body(Body::from("No Avaialble RPC Urls"))
                .unwrap());
        }
    }
}

async fn retry_with_backoff(
    method: Arc<Method>,
    body_bytes: Arc<Bytes>,
    state: Arc<Mutex<RoundRobin>>,
) -> Option<ReqwestResponse> {
    let mut retries: u32 = 0;
    let base_delay = Duration::from_millis(100);

    let max_retries;

    {
        let rr = state.lock().unwrap();
        max_retries = rr.urls.len() as u32;
    }

    while retries < max_retries {
        let result = get_forward_request(state.clone(), method.clone(), body_bytes.clone()).await;

        if let Some(request) = result {
            if let Ok(res) = request.send().await {
                if res.status().is_success() {
                    return Some(res);
                }
            }
        }

        {
            let round_robin = state.lock().unwrap();
            round_robin.retry_connection();
        }

        retries += 1;
        if retries < max_retries {
            let current_delay = base_delay * 2_u32.pow(retries);
            println!("Retrying with another RPC Url in {:?}.", current_delay);
            tokio::time::sleep(current_delay).await;
        }
    }

    None
}

async fn get_forward_request(
    state: Arc<Mutex<RoundRobin>>,
    method: Arc<Method>,
    body_bytes: Arc<Bytes>,
) -> Option<RequestBuilder> {
    let uri;

    {
        let mut round_robin = state.lock().unwrap();
        uri = round_robin.get_next();
    }

    if let Some(uri) = uri {
        println!("Forwarding request to : {}", &uri);

        let client = reqwest::Client::new();

        let mut forwarded_request = client.request((*method).clone(), &uri);

        forwarded_request = forwarded_request.header("Content-Type", "application/json");
        forwarded_request = forwarded_request.body((*body_bytes).clone());
        return Some(forwarded_request);
    } else {
        None
    }
}

// load balancer tests
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
        let response = load_balancer(path, State(Arc::new(lbs)), request)
            .await
            .unwrap();
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

        let response = load_balancer(path, State(Arc::new(lbs)), request)
            .await
            .unwrap();

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
            RpcServer {
                url: "https://endpoints.omniatech.io/v1/eth/sepolia/public".to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let mock_round_robin = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("ethereum_sepolia".to_string(), mock_round_robin);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };
        let path: Path<String> = Path("ethereum_sepolia".to_string());
        println!("before resp");
        let response = load_balancer(path, State(Arc::new(lbs)), request)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    async fn test_multiple_chains() {
        let request = create_test_request();
        let servers = vec![
            RpcServer {
                url: "https://eth-sepolia.g.alchemy.com/v2/mRRENj5uQ1jqgfIIrtFZFzqUWQtU1lvH"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://eth-sepolia.g.alchemy.com/v2/fjZ8CPTHtjIN989lInvYqljpGNqJTspg"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let arb = vec![
            RpcServer {
                url: "https://arb-sepolia.g.alchemy.com/v2/DumcaFO69U55TqhPevuTScTlDzxhvy0N"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://arb-sepolia.g.alchemy.com/v2/Vt-glQ2N0u8FIs-f0try1ghd7DAdYobc"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let base = vec![
            RpcServer {
                url: "https://base-sepolia.g.alchemy.com/v2/DumcaFO69U55TqhPevuTScTlDzxhvy0N"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://base-sepolia.g.alchemy.com/v2/Vt-glQ2N0u8FIs-f0try1ghd7DAdYobc"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let berachain = vec![
            RpcServer {
                url: "https://berachain-bartio.g.alchemy.com/v2/DumcaFO69U55TqhPevuTScTlDzxhvy0N"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
            RpcServer {
                url: "https://berachain-bartio.g.alchemy.com/v2/mRRENj5uQ1jqgfIIrtFZFzqUWQtU1lvH"
                    .to_string(),
                request_limit: 1,
                current_limit: 1,
            },
        ];

        let bitcoin = vec![
            RpcServer{
                url : "https://rpc.ankr.com/btc_signet/2a8161e0d7bc03b1d7198e539c94b34481ad94443090a041314aedc2b29ea17b".to_string(),
                request_limit : 5,
                current_limit : 5
            },
            RpcServer{
                url : "https://rpc.ankr.com/btc_signet/bc0fb296415993c1eccfc983e9b8f4881272efa66f8f92fa916ea053b2bb768c".to_string(),
                request_limit : 5,
                current_limit : 5 },
        ];

        let sepolia_servers = Arc::new(Mutex::new(RoundRobin::new(servers)));
        let arb_servers = Arc::new(Mutex::new(RoundRobin::new(arb)));
        let base_servers = Arc::new(Mutex::new(RoundRobin::new(base)));
        let berachain_servers = Arc::new(Mutex::new(RoundRobin::new(berachain)));
        let bitcoin_servers = Arc::new(Mutex::new(RoundRobin::new(bitcoin)));
        let mut chains: HashMap<String, Arc<Mutex<RoundRobin>>> = HashMap::new();
        chains.insert("ethereum_sepolia".to_string(), sepolia_servers);
        chains.insert("arbitrum_sepolia".to_string(), arb_servers);
        chains.insert("base_sepolia".to_string(), base_servers);
        chains.insert("berachain".to_string(), berachain_servers);
        chains.insert("bitcoin".to_string(), bitcoin_servers);
        let fin_chains = Arc::new(chains);
        let lbs = LoadBalancer {
            load_balancers: fin_chains,
        };

        {
            let round_robin_lb = &lbs.load_balancers;

            for round_robin in round_robin_lb.values() {
                let rr_clone;
                {
                    let rr = round_robin.lock().unwrap();
                    rr_clone = rr.clone();
                }

                tokio::spawn(async move {
                    rr_clone.refill_limits(Duration::from_secs(5)).await;
                });
            }
        }

        let path: Path<String> = Path("ethereum_sepolia".to_string());
        let response = load_balancer(path, State(Arc::new(lbs.clone())), request)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);

        let req2 = create_test_request();
        let path: Path<String> = Path("base_sepolia".to_string());
        let response = load_balancer(path, State(Arc::new(lbs.clone())), req2)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);

        let req3 = create_test_request();
        let path: Path<String> = Path("arbitrum_sepolia".to_string());
        let response = load_balancer(path, State(Arc::new(lbs.clone())), req3)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);

        let req4 = create_test_request();
        let path: Path<String> = Path("berachain".to_string());
        let response = load_balancer(path, State(Arc::new(lbs.clone())), req4)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);

        let req5 = create_test_request();
        let path: Path<String> = Path("bitcoin".to_string());
        let response = load_balancer(path, State(Arc::new(lbs.clone())), req5)
            .await
            .unwrap();
        println!("{}", response.status());
        assert_eq!(response.status(), StatusCode::OK);
    }
}
