use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::Deserialize;
use tokio::time;

#[derive(Clone, Debug)]
pub struct RoundRobin {
    pub urls: Vec<RpcServer>,
    pub index: Arc<AtomicUsize>,
}
impl RoundRobin {
    pub fn new(urls: Vec<RpcServer>) -> Self {
        Self {
            urls: urls,
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_next(&mut self) -> Option<String> {
        let len = self.urls.len();
        for _ in 0..len {
            let i = self.index.load(Ordering::Relaxed);

            if self.urls[i].current_limit > 0 {
                self.urls[i].current_limit -= 1;
                return Some(self.urls[i].url.clone());
            }
            self.index.store((i + 1) % len, Ordering::Relaxed);
        }

        // If no servers have available limits, return None
        None
    }

    pub async fn refill_limits(&mut self) {
        loop {
            for server in &mut self.urls {
                server.current_limit = server.request_limit;
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub fn retry_connection(&self) {
        let len = self.urls.len();
        let i = self.index.load(Ordering::Relaxed);
        self.index.store((i + 1) % len, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct LoadBalancer {
    pub load_balancers: Arc<HashMap<String, Arc<Mutex<RoundRobin>>>>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub chains: HashMap<String, Chains>,
}

#[derive(Deserialize, Debug)]
pub struct Chains {
    pub rpc_urls: Vec<RpcServer>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct RpcServer {
    pub url: String,
    pub current_limit: u32,
    pub request_limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

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

    #[test]
    fn test_new_round_robin() {
        let servers = create_test_servers();
        let round_robin = RoundRobin::new(servers.clone());

        assert_eq!(round_robin.urls.len(), servers.len());

        let index = round_robin.index.load(Ordering::Relaxed);
        assert_eq!(index, 0);

        for (i, server) in round_robin.urls.iter().enumerate() {
            assert_eq!(server.url, servers[i].url);
            assert_eq!(server.request_limit, servers[i].request_limit);
            assert_eq!(server.current_limit, servers[i].current_limit);
        }
    }

    #[test]
    fn test_get_next() {
        let servers = create_test_servers();
        let mut round_robin = RoundRobin::new(servers);

        let url1 = round_robin.get_next();
        assert_eq!(url1, Some("https://sepolia.drpc.org/".to_string()));
        assert_eq!(round_robin.index.load(Ordering::Relaxed), 0);

        let url2 = round_robin.get_next();
        assert_eq!(url2, Some("https://polygon-rpc.com".to_string()));
        assert_eq!(round_robin.index.load(Ordering::Relaxed), 1);

        let url3 = round_robin.get_next();
        assert_eq!(url3, None);
        assert_eq!(round_robin.index.load(Ordering::Relaxed), 1);

        let url4 = round_robin.get_next();
        assert_eq!(url4, None);
        assert_eq!(round_robin.index.load(Ordering::Relaxed), 1);
    }
}
