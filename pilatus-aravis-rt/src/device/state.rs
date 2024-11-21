use futures::StreamExt;
use pilatus::device::ActorResult;
use pilatus_aravis::{RunningState, SubscribeRunningStateMessage};
use tokio::sync::watch;

impl super::State {
    pub(super) async fn subscribe_state(
        &mut self,
        _msg: SubscribeRunningStateMessage,
    ) -> ActorResult<SubscribeRunningStateMessage> {
        Ok(tokio_stream::wrappers::WatchStream::new(self.state.watch.subscribe()).boxed())
    }
}

#[derive(Clone)]
pub(super) struct State {
    watch: watch::Sender<RunningState>,
}

impl State {
    pub(super) fn publish_if_changed(&self, new_state: RunningState) {
        self.watch.send_if_modified(|cur| {
            if cur == &new_state {
                false
            } else {
                *cur = new_state;
                true
            }
        });
    }
}

impl Default for State {
    fn default() -> Self {
        let (watch, _) = watch::channel(RunningState::NotConnected);
        Self { watch }
    }
}
