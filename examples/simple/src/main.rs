// cargo run --example simple --release
// visit "http://localhost/api/image/viewer"

use std::time::Duration;

use futures::StreamExt;
use minfac::{Registered, ServiceCollection};
use pilatus::{device::ActorSystem, prelude::HostedServiceServiceServiceBuilderExtensions};
use pilatus_rt::Runtime;

fn main() {
    Runtime::default()
        .register(pilatus_engineering_rt::register)
        .register(pilatus_axum_rt::register)
        .register(pilatus_aravis::register)
        .register(register)
        .run();
}

extern "C" fn register(c: &mut ServiceCollection) {
    c.register(|| {
        pilatus::InitRecipeListener::new(|r| {
            r.add_device(pilatus_aravis::create_default_camera_config());
        })
    });
    c.with::<Registered<ActorSystem>>()
        .register_hosted_service("state_watcher", |s| async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let mut state_stream = std::pin::pin!(
                s.get_sender_or_single_handler(None)?
                    .ask(pilatus_aravis::SubscribeRunningStateMessage::default())
                    .await?
            );
            while let Some(s) = state_stream.next().await {
                tracing::info!("Camera changed state: {:?}", s);
            }
            tracing::debug!("Stop watching state");
            Ok(())
        });
}
