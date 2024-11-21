use pilatus::SubscribeMessage;

pub type SubscribeRunningStateMessage = SubscribeMessage<(), RunningState, ()>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningState {
    NotConnected,
    Error,
    Running,
}
