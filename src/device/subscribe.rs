use std::{
    num::Saturating,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures::{Stream, StreamExt, TryStreamExt};
use pilatus::{device::ActorResult, MissedItemsError};
use pilatus_engineering::image::{
    BroadcastImage, DynamicImage, ImageWithMeta, SubscribeDynamicImageMessage,
    SubscribeImageMessage,
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{debug, warn};

use crate::wrapper::StreamingAction;

impl super::State {
    pub(super) async fn subscribe(
        &mut self,
        _msg: SubscribeImageMessage,
    ) -> ActorResult<SubscribeImageMessage> {
        Ok(self
            .subscribe_broadcast()
            .await
            .map(|img| match img {
                Ok(ImageWithMeta {
                    image: pilatus_engineering::image::DynamicImage::Luma8(img),
                    meta,
                    ..
                }) => Some(BroadcastImage::with_hash(Arc::new(img), meta.hash)),
                e => {
                    warn!("Camera produces images of a format which was not requested: {e:?}");
                    None
                }
            })
            .take_while(|x| std::future::ready(x.is_some()))
            .map(|x| x.expect("checked above"))
            .boxed())
    }

    pub(super) async fn subscribe_dynamic(
        &mut self,
        _msg: SubscribeDynamicImageMessage,
    ) -> ActorResult<SubscribeDynamicImageMessage> {
        Ok(self.subscribe_broadcast().await.map_err(Into::into).boxed())
    }

    async fn subscribe_broadcast(
        &mut self,
    ) -> impl Stream<Item = Result<ImageWithMeta<DynamicImage>, MissedItemsError>> {
        let mapper = |e| {
            let BroadcastStreamRecvError::Lagged(e) = e;
            MissedItemsError::new(Saturating(e.max(u16::MAX as _) as u16))
        };
        if let Some(stream) = self.running.take() {
            let r = stream.broadcast.subscribe();
            // Continues running until stopped or all receiver terminate
            // 1. Receiver was not stopped, as there was another receiver before. Otherwise shutdown anyway to make sure
            // 2. We did not actively shutdown stream
            if stream.broadcast.receiver_count() > 1 {
                self.running = Some(stream);
                return tokio_stream::wrappers::BroadcastStream::new(r).map_err(mapper);
            } else {
                // Changes are, that the thread started terminating
                stream.shutdown().await;
            }
        };

        debug!("Start new Image stream with aravis");
        let (sender, receiver) = broadcast::channel(1);
        let builder = self
            .factory
            .create_builder(self.params.identifier.clone())
            .with_features(self.params.features.clone());
        let broadcast = sender.clone();
        let should_terminate = Arc::new(AtomicBool::new(false));
        let should_terminate_copy = should_terminate.clone();
        let (async_term_send, async_thread_termination) = futures::channel::oneshot::channel();

        self.running = Some(RunningState {
            should_terminate,
            broadcast,
            async_thread_termination,
            thread_handle: std::thread::spawn(move || loop {
                let r = builder
                    .clone()
                    .with_termination(should_terminate_copy.clone())
                    .build(|img| {
                        sender
                            .send(img)
                            .map(|x| {
                                if x > 0 {
                                    StreamingAction::Continue
                                } else {
                                    StreamingAction::Stop
                                }
                            })
                            .unwrap_or(StreamingAction::Stop)
                    });

                match r {
                    Ok(StreamingAction::Stop) => {
                        debug!("Camera finished streaming");
                        drop(async_term_send);
                        break;
                    }
                    Ok(StreamingAction::Continue) => {}
                    Err(e) => {
                        debug!("Error during acquisition: {e:?}");
                        if should_terminate_copy.load(Ordering::Relaxed) {
                            break;
                        } else {
                            std::thread::sleep(Duration::from_secs(1))
                        }
                    }
                }
            }),
        });
        tokio_stream::wrappers::BroadcastStream::new(receiver).map_err(mapper)
    }

    pub async fn stop_streaming(&mut self) {
        if let Some(s) = self.running.take() {
            s.shutdown().await
        }
    }
}

pub(super) struct RunningState {
    broadcast: broadcast::Sender<ImageWithMeta<DynamicImage>>,
    thread_handle: std::thread::JoinHandle<()>,
    async_thread_termination: futures::channel::oneshot::Receiver<()>,
    should_terminate: Arc<AtomicBool>,
}

impl RunningState {
    async fn shutdown(self) {
        self.should_terminate.store(true, Ordering::Relaxed);
        self.async_thread_termination.await.ok();
        self.thread_handle
            .join()
            .expect("Camera thread never panicks");
    }
}
