use minfac::{Registered, ServiceCollection};
use pilatus::{
    device::{
        ActorResult, ActorSystem, DeviceContext, DeviceResult, DeviceValidationContext,
        ServiceBuilderExtensions,
    },
    DeviceConfig, GenericConfig, UpdateParamsMessage, UpdateParamsMessageError,
};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::wrapper::CameraFactory;

mod subscribe;

const DEVICE_TYPE: &str = "pilatus-camera-aravis";

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Params {
    identifier: Option<String>,
    features: crate::genicam::GenicamFeatureCollection,
}

struct State {
    params: Params,
    running: Option<subscribe::RunningState>,
    factory: CameraFactory,
}

pub(super) fn register_services(c: &mut ServiceCollection) {
    c.with::<(
        Registered<ActorSystem>,
        Registered<GenericConfig>,
        Registered<CameraFactory>,
    )>()
    .register_device(DEVICE_TYPE, validator, device);
}

async fn validator(ctx: DeviceValidationContext<'_>) -> Result<Params, UpdateParamsMessageError> {
    ctx.params_as::<Params>()
}

async fn device(
    ctx: DeviceContext,
    params: Params,
    (actor_system, _config, factory): (ActorSystem, GenericConfig, CameraFactory),
) -> DeviceResult {
    let actor = actor_system
        .register(ctx.id)
        .add_handler(State::update_params)
        .add_handler(State::subscribe_dynamic)
        .add_handler(State::subscribe);

    let list_factory = factory.clone();
    tokio::task::spawn_blocking(move || debug!("Devicelist: {:?}", list_factory.get_device_list()))
        .await?;

    let mut state = actor
        .execute(State {
            params,
            running: Default::default(),
            factory,
        })
        .await;
    state.stop_streaming().await;

    Ok(())
}

impl State {
    async fn update_params(
        &mut self,
        _msg: UpdateParamsMessage<Params>,
    ) -> ActorResult<UpdateParamsMessage<Params>> {
        Ok(())
    }
}
pub fn create_default_camera_config() -> DeviceConfig {
    DeviceConfig::new_unchecked(DEVICE_TYPE, "camera", Params::default())
}
