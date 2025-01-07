use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde::Deserialize;

use crate::models::rpc_model::RpcServer;

#[derive(Clone, Debug, Default)]
pub struct RoundRobin {
    pub urls: Arc<Vec<RpcServer>>,
    pub index: Arc<AtomicUsize>,
}
impl RoundRobin {
    pub fn new(urls: Vec<RpcServer>) -> Self {
        Self {
            urls: Arc::new(urls),
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_next(&self) -> String {
        let i = self.index.fetch_add(1, Ordering::Relaxed) % self.urls.len();
        self.urls[i].url.clone()
    }
}
