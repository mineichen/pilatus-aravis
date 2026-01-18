use pilatus::SubscribeMessage;
use serde::{Deserialize, Serialize};

pub type SubscribeRunningStateMessage = SubscribeMessage<(), CameraStatus, ()>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraStatus {
    NotConnected,
    Error,
    Running,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub physical_id: String,
    pub vendor: String,
    pub model: String,
    pub protocol: String,
    pub address: String,
}
