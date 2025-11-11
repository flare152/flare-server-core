use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub service: ServiceConfig,
    pub server: ServerConfig,
    pub registry: Option<RegistryConfig>,
    pub mesh: Option<MeshConfig>,
    pub storage: Option<StorageConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub storage_type: String, // minio, s3, tencent, alibaba
    pub endpoint: Option<String>,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub bucket: Option<String>,
    pub region: Option<String>,
    pub use_ssl: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_type")]
    pub registry_type: String, // etcd, consul, mesh
    pub endpoints: Vec<String>,
    pub namespace: String,
    pub ttl: u64,
}

fn default_registry_type() -> String {
    "etcd".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeshConfig {
    pub enabled: bool,
    pub service_name: String,
    pub namespace: String,
}

impl Config {
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
