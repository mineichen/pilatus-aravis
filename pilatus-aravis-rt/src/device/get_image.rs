use futures::StreamExt;
use pilatus::device::ActorResult;
use pilatus_engineering::image::{GetImageMessage, ImageWithMeta, SubscribeImageMessage};

impl super::State {
    pub(super) async fn handle_get_image(
        &mut self,
        _msg: GetImageMessage,
    ) -> ActorResult<GetImageMessage> {
        let image = self
            .acquire(SubscribeImageMessage::default())
            .await
            .map_err(anyhow::Error::from)?
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("Empty stream"))?
            .image;
        Ok(ImageWithMeta::with_hash(image, None))
    }
}
