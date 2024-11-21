use std::{
    num::Saturating,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::{Stream, StreamExt, TryStreamExt};
use pilatus::{device::ActorResult, MissedItemsError, SubscribeMessage};
use pilatus_engineering::image::{
    BroadcastImage, DynamicImage, ImageWithMeta, SubscribeDynamicImageMessage,
    SubscribeImageMessage,
};
use tokio::sync::watch;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{debug, warn};

use crate::wrapper::StreamingAction;

pub type SubscribeRunningStateMessage = SubscribeMessage<(), RunningState, ()>;

impl super::State {
    pub(super) async fn subscribe_state(
        &mut self,
        msg: SubscribeRunningStateMessage,
    ) -> ActorResult<SubscribeRunningStateMessage> {
        Ok(tokio_stream::wrappers::WatchStream::new(self.state.watch.subscribe()).boxed())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningState {
    NotConnected,
    Error,
    Running,
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
