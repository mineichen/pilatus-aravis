use std::{
    ffi::{CStr, CString},
    str::Utf8Error,
    sync::Arc,
};

use aravis::Aravis;
use minfac::ServiceCollection;
use pilatus_aravis::DeviceInfo;
use pilatus_axum::{
    extract::{InjectRegistered, Json, Path},
    http::StatusCode,
    IntoResponse, ServiceCollectionExtensions,
};
use tracing::error;

pub(super) fn register_services(c: &mut ServiceCollection) {
    #[rustfmt::skip]
    c.register_web(crate::device::DEVICE_TYPE, |x| x
        .http("/camera", |x| x.get(list_cameras))
        .http("/camera/{identifier}", |x| x.put(change_camera_ip))
    );
}

async fn change_camera_ip(
    Path(_identifier): Path<String>,
    InjectRegistered(a): InjectRegistered<Arc<Aravis>>,
) -> impl IntoResponse {
    //let device = aravis::Device::new(identifier.as_str()).unwrap();
}

async fn list_cameras(
    InjectRegistered(x): InjectRegistered<Arc<Aravis>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cameras = tokio::task::spawn_blocking(move || x.get_device_list())
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to spawn blocking task".to_string(),
            )
        })?;
    Ok(Json(
        cameras
            .iter()
            .map(|x| {
                let extract = |value: &CStr, info| {
                    value.to_str().map(|x| x.to_string()).map_err(|x| (x, info))
                };
                Ok(DeviceInfo {
                    id: extract(&x.id, x)?,
                    physical_id: extract(&x.physical_id, x)?,
                    vendor: extract(&x.vendor, x)?,
                    model: extract(&x.model, x)?,
                    protocol: extract(&x.protocol, x)?,
                    address: extract(&x.address, x)?,
                })
            })
            .filter_map(|r| {
                r.inspect_err(|(e, info)| error!("Skipping Item {info:?}: {}", e))
                    .ok()
            })
            .collect::<Vec<_>>(),
    ))
}
