use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub settings: Settings,
    pub rpc_urls: RpcUrls,
}

#[derive(Deserialize, Debug)]
pub struct Settings {
    pub retry_count: u32,
    pub timeout: u32,
    pub log_level: String,
}

#[derive(Deserialize, Debug)]
pub struct RpcUrls {
    pub server: Vec<RpcServer>,
}

#[derive(Deserialize, Debug)]
pub struct RpcServer {
    pub url: String,
    pub weight: u32,
    pub chain_id: String,
    pub max_connections: u32,
}
