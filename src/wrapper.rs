use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::anyhow;
use aravis::{
    AcquisitionMode, Aravis, Buffer, BufferPayloadType, BufferStatus, CameraExt, CameraExtManual,
    StreamExt,
};
use futures::stream::FusedStream;
use minfac::{Registered, ServiceCollection};
use tracing::{debug, error, info, trace};

use crate::{genicam::GenicamFeatureCollection, ReturnableBuffer, ToPilatusImageExt};

pub(super) fn register_services(c: &mut ServiceCollection) {
    c.register_shared(|| {
        Arc::new(aravis::Aravis::initialize().expect("Noone else is allowed to initialize"))
    });
    c.with::<Registered<Arc<Aravis>>>()
        .register(|ctx| CameraFactory { ctx });
}

#[derive(Clone)]
pub struct CameraFactory {
    ctx: Arc<aravis::Aravis>,
}

#[derive(Clone)]
pub struct CameraBuilder {
    camera_identifier: Option<String>,
    features: GenicamFeatureCollection,
    is_termination_requested: Arc<AtomicBool>,
}

pub enum StreamingAction {
    Continue,
    Stop,
}

impl CameraFactory {
    pub fn get_device_list(&self) -> Vec<aravis::DeviceInfo> {
        self.ctx.get_device_list()
    }
    pub fn create_builder(&self, camera_identifier: Option<String>) -> CameraBuilder {
        CameraBuilder {
            camera_identifier,
            features: Default::default(),
            is_termination_requested: Arc::new(AtomicBool::new(false)),
        }
    }
    #[cfg(test)]
    pub fn build(
        &self,
        callback: impl FnMut(pilatus_engineering::image::DynamicImage) -> StreamingAction,
    ) -> anyhow::Result<StreamingAction> {
        CameraBuilder {
            camera_identifier: None,
            features: Default::default(),
            is_termination_requested: Arc::new(AtomicBool::new(false)),
        }
        .build(callback)
    }
}

impl CameraBuilder {
    pub fn with_features(mut self, features: GenicamFeatureCollection) -> Self {
        self.features = features;
        self
    }
    pub fn with_termination(mut self, is_termination_requested: Arc<AtomicBool>) -> Self {
        self.is_termination_requested = is_termination_requested;
        self
    }
    pub fn build(
        self,
        mut callback: impl FnMut(pilatus_engineering::image::DynamicImage) -> StreamingAction,
    ) -> anyhow::Result<StreamingAction> {
        let mut camera = aravis::Camera::new(self.camera_identifier.as_deref())?;
        let props = get_features(&camera)?.collect::<Vec<_>>();
        if self.is_termination_requested.load(Ordering::Relaxed) {
            return Ok(StreamingAction::Stop);
        }

        info!("Create Camera with features {:?}", props);
        let num_features = self.features.apply(&mut camera)?;

        debug!("Camera is created with {num_features} features");
        camera.set_acquisition_mode(AcquisitionMode::Continuous)?;
        let stream = camera.create_stream()?;

        let size = camera.payload()?;

        let (send_buf_back, mut recv_buffer) = futures::channel::mpsc::channel(10);

        for _ in 0..2 {
            stream.push_buffer(Buffer::new_leaked_box(size as _));
        }

        camera.start_acquisition()?;

        let mut last = std::time::Instant::now();
        loop {
            trace!("Before pop_buffer");
            let Some(mut buf) = stream.timeout_pop_buffer(5_100_000) else {
                drop(stream);
                return Err(anyhow!(
                    "A long time elapsed without images: {:?}",
                    last.elapsed()
                ));
            };
            trace!("After pop_buffer");
            if self.is_termination_requested.load(Ordering::Relaxed) {
                break;
            }
            let elapsed = last.elapsed();
            last = std::time::Instant::now();
            if elapsed > Duration::from_secs(5) {}
            // let pilatus_image = convert_buf.try_into_pilatus_luma();
            // stream.push_buffer(&buf);
            if buf.status() != BufferStatus::Success {
                debug!("{elapsed:?} invalid buffer received: {:?}", buf.status());
                // returning existing buffer caused segfault
                //stream.push_buffer(Buffer::new_leaked_box(size as _));
                stream.push_buffer(buf);

                if recv_buffer.is_terminated() {
                    break;
                } else {
                    trace!("Continue with stream");
                    continue;
                }
            }
            if !matches!(
                buf.payload_type(),
                BufferPayloadType::Image | BufferPayloadType::Unknown
            ) {
                trace!(
                    "{elapsed:?} unexpected payload: {:?} of size {}",
                    buf.payload_type(),
                    buf.data().1
                );
                // return existing buf caused deadlock
                stream.push_buffer(buf);
                if recv_buffer.is_terminated() {
                    break;
                } else {
                    continue;
                }
            }
            let pixel_format = buf.image_pixel_format();

            let mut convert_buf = match recv_buffer.try_next() {
                Ok(Some(b)) => b,
                _ => Box::new(ReturnableBuffer::new(
                    Buffer::new_allocate(size as _),
                    send_buf_back.clone(),
                )),
            };
            convert_buf.swap_buf(&mut buf);
            trace!("{elapsed:?} {pixel_format:?}");

            let pilatus_image = match pixel_format.raw() {
                aravis_sys::ARV_PIXEL_FORMAT_MONO_8 => convert_buf
                    .try_into_pilatus()
                    .map(pilatus_engineering::image::DynamicImage::Luma8),

                aravis_sys::ARV_PIXEL_FORMAT_MONO_16 | crate::PIXELFORMAT_COORD3D_C16 => {
                    convert_buf
                        .try_into_pilatus()
                        .map(pilatus_engineering::image::DynamicImage::Luma16)
                }

                e => {
                    error!("Can't handle pixel format {e:?}");
                    break;
                }
            }; /*
               if let Ok(x) = pilatus_image.as_ref() {
                   let clone = x.clone();
                   std::thread::spawn(move || match clone {
                       pilatus_engineering::image::DynamicImage::Luma8(img) => println!("I8image"),
                       pilatus_engineering::image::DynamicImage::Luma16(img) => {
                           let mut file = std::fs::File::create(format!(
                               "{}.png",
                               (std::time::SystemTime::now()
                                   .duration_since(std::time::SystemTime::UNIX_EPOCH)
                                   .unwrap()
                                   .as_millis())
                           ))
                           .unwrap();
                           let encoder = image::codecs::png::PngEncoder::new(file);
                           let (width, height) = img.dimensions();
                           let slice = unsafe {
                               let len = img.buffer().len();
                               std::slice::from_raw_parts(img.buffer().as_ptr().cast::<u8>(), 2 * len)
                           };
                           encoder
                               .write_image(
                                   slice,
                                   width.get() as _,
                                   height.get() as _,
                                   image::ExtendedColorType::L16,
                               )
                               .unwrap();
                       }
                       _ => todo!(),
                   });
               }*/

            // let (ptr, len) = buf.data();
            // let width = buf.image_width() as u32;
            // let height = buf.image_height() as u32;

            // let vec = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();

            // let pilatus_image =
            //     anyhow::Ok(LumaImage::new(vec, width.try_into()?, height.try_into()?));
            stream.push_buffer(buf);

            match (callback)(pilatus_image?) {
                StreamingAction::Continue => {}
                StreamingAction::Stop => return Ok(StreamingAction::Stop),
            }
        }

        debug!("Running out of buffers?");
        Ok(StreamingAction::Stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::StreamExt;
    use image::Luma;

    #[tokio::test]
    #[ignore = "requires hardware"]
    async fn it_works() -> anyhow::Result<()> {
        let mut collection = ServiceCollection::new();
        register_services(&mut collection);
        let provider = collection.build().expect("No external dependencies");
        let factory: CameraFactory = provider.get().unwrap();

        let (mut sender, receiver) = futures::channel::mpsc::channel(10);
        let mut ctr = 0;
        let producer_join = std::thread::spawn(move || {
            factory
                .build(|img| {
                    ctr += 1;
                    if sender.try_send(img).is_err() {
                        return StreamingAction::Stop;
                    }
                    (ctr < 10)
                        .then_some(StreamingAction::Continue)
                        .unwrap_or(StreamingAction::Stop)
                })
                .unwrap();
        });

        let path = std::path::Path::new("data");
        tokio::fs::create_dir_all(path).await?;
        let mut consumer_joins = Vec::new();
        let mut stream = receiver.enumerate();
        while let Some((i, pilatus_engineering::image::DynamicImage::Luma8(img))) =
            stream.next().await
        {
            consumer_joins.push(std::thread::spawn(move || {
                let (width, height) = img.dimensions();
                let f = image::ImageBuffer::<Luma<u8>, _>::from_raw(
                    width.get(),
                    height.get(),
                    img.buffer(),
                )
                .unwrap();
                for _ in 0..10 {
                    f.save(path.join(&format!("testimage_{i}.png")))
                        .expect("save failed");
                }
            }));
        }

        producer_join.join().unwrap();
        for handle in consumer_joins {
            handle.join().unwrap();
        }
        Ok(())
    }
}

fn get_features(_camera: &aravis::Camera) -> anyhow::Result<impl Iterator<Item = String>> {
    // let (root, _) = camera
    //     .device()
    //     .ok_or_else(|| anyhow!("No device available in aravis::Camera"))?
    //     .genicam_xml();
    // let mut node_store = DefaultNodeStore::new();
    // let mut value_builder = DefaultValueStore::new();
    // let mut cache_builder = DefaultCacheStore::new();
    // let x = cameleon_genapi::parser::parse(
    //     &root,
    //     &mut node_store,
    //     &mut value_builder,
    //     &mut cache_builder,
    // )?;

    Ok(
        std::iter::empty(), /*(0..len).filter_map(move |x| {
                                let item = props.item(x)?;
                                dbg!(item.node_value(), item.node_type());

                                item.node_name().map(|x| dbg!(x).to_string())
                            })*/
    )
}
