use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub service_type: String,
    pub service_id: String,
    pub instance_id: String,
    pub address: String,
    pub port: u16,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceType {
    Signaling,
    Push,
    Storage,
    Business,
}
