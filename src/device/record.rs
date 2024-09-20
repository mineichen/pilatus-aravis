use pilatus::device::{ActorMessage, ActorResult};
use pilatus_engineering::image::{DynamicImage, StreamImageError, SubscribeDynamicImageMessage};

impl super::State {
    pub(super) async fn record(
        &mut self,
        _msg: RecordStreamMessage,
    ) -> ActorResult<RecordStreamMessage> {
        Ok(())
    }
}

pub struct RecordStreamMessage {}

impl ActorMessage for RecordStreamMessage {
    type Output = ();
    type Error = StreamImageError<DynamicImage>;
}
