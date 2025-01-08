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

#[derive(Deserialize, Debug)]
pub struct RpcServer {
    pub url: String,
    pub current_limit: u32,
    pub request_limit: u32,
}
