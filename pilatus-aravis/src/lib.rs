use pilatus::SubscribeMessage;

pub type SubscribeRunningStateMessage = SubscribeMessage<(), CameraStatus, ()>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraStatus {
    NotConnected,
    Error,
    Running,
}
