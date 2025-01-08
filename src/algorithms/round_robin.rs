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
    pub urls: Arc<Vec<Mutex<RpcServer>>>,
    pub index: Arc<AtomicUsize>,
}
impl RoundRobin {
    pub fn new(urls: Vec<RpcServer>) -> Self {
        let urls = urls.into_iter().map(Mutex::new).collect();
        Self {
            urls: Arc::new(urls),
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_next(&self) -> Option<String> {
        let len = self.urls.len();
        for _ in 0..len {
            let i = self.index.load(Ordering::Relaxed) % self.urls.len();
            let mut server = self.urls[i].lock().unwrap();
            println!(
                "inside get next printing server currentname {} and curr_limit {}",
                server.url, server.current_limit
            );
            if server.current_limit > 0 {
                server.current_limit -= 1;
                return Some(server.url.clone());
            }
            self.index.store((i + 1) % len, Ordering::Relaxed);
        }

        // If no servers have available limits, return None
        None
    }

    pub async fn refill_limits(&self) {
        loop {
            println!("{:?}", self.urls);
            for server in self.urls.iter() {
                {
                    let mut server = server.lock().unwrap();
                    server.current_limit = server.request_limit;
                }
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub fn retry_connection(&self) -> Option<String> {
        let len = self.urls.len();
        let i = self.index.load(Ordering::Relaxed);
        self.index.store((i + 1) % len, Ordering::Relaxed);

        return self.get_next();
    }
}

#[derive(Debug)]
pub struct LoadBalancer {
    pub load_balancers: Arc<Mutex<HashMap<String, Arc<Mutex<RoundRobin>>>>>,
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
    #[ignore]
    fn test_new_round_robin() {
        let servers = create_test_servers();
        let round_robin = RoundRobin::new(servers.clone());

        assert_eq!(round_robin.urls.len(), servers.len());

        let index = round_robin.index.load(Ordering::Relaxed);
        assert_eq!(index, 0);

        for (i, server) in round_robin.urls.iter().enumerate() {
            let server = server.lock().unwrap();
            assert_eq!(server.url, servers[i].url);
            assert_eq!(server.request_limit, servers[i].request_limit);
            assert_eq!(server.current_limit, servers[i].current_limit);
        }
    }

    #[test]
    #[ignore]
    fn test_get_next() {
        let servers = create_test_servers();
        let round_robin = RoundRobin::new(servers);

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
