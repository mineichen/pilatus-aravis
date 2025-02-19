use std::{
    borrow::Cow, collections::{btree_map::Entry, BTreeMap, HashMap}, mem::MaybeUninit, sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    }
};

use anyhow::{anyhow, Context};
use aravis::{
    AcquisitionMode, Aravis, Buffer, BufferPartDataType, BufferPayloadType, BufferStatus,
    CameraExt, CameraExtManual, StreamExt,
};
use futures::stream::FusedStream;
use minfac::{Registered, ServiceCollection};
use pilatus_engineering::image::{DynamicImage, GenericImage, ImageMeta, ImageWithMeta, SpecificImageKey};
use tracing::{debug, info, trace, warn};

use crate::{
    buffer::ReturnableBuffer, genicam::GenicamFeatureCollection, try_into_dynamic_pilatus_image,
};

pub(super) fn register_services(c: &mut ServiceCollection) {
    c.register(|| {
        static INSTANCE: LazyLock<Arc<Aravis>> = std::sync::LazyLock::new(|| {
            Arc::new(aravis::Aravis::initialize().expect("Noone else is allowed to initialize"))
        });

        INSTANCE.clone()
    });
    c.with::<Registered<Arc<Aravis>>>()
        .register(|ctx| CameraFactory { ctx });
}

#[derive(Clone)]
pub struct CameraFactory {
    ctx: Arc<aravis::Aravis>,
}

//#[derive(Clone)]
pub struct CameraRunner {
    callback: Box<dyn FnMut(&mut aravis::Camera) -> anyhow::Result<()> + Send + Sync + 'static>,
    camera_identifier: Option<String>,
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
    pub fn create_runner(&self, camera_identifier: Option<String>) -> CameraRunner {
        CameraRunner {
            callback: Box::new(|_| Ok(())),
            camera_identifier,
            is_termination_requested: Arc::new(AtomicBool::new(false)),
        }
    }
    #[cfg(test)]
    pub fn run(
        &self,
        callback: impl FnMut(ImageWithMeta<DynamicImage>) -> StreamingAction,
    ) -> anyhow::Result<StreamingAction> {
        CameraRunner {
            callback: Box::new(|_| Ok(())),
            camera_identifier: None,
            is_termination_requested: Arc::new(AtomicBool::new(false)),
        }
        .run(callback)
    }
}

impl CameraRunner {
    pub fn with_termination(mut self, is_termination_requested: Arc<AtomicBool>) -> Self {
        self.is_termination_requested = is_termination_requested;
        self
    }

    pub fn on_connect(mut self, callback: impl FnMut(&mut aravis::Camera) -> anyhow::Result<()> + Send + Sync + 'static) -> Self {
        self.callback = Box::new(callback);
        self
    }

    pub fn run(
        &mut self,
        mut callback: impl FnMut(
            ImageWithMeta<DynamicImage>,
        ) -> StreamingAction,
    ) -> anyhow::Result<StreamingAction> {
        let mut camera = aravis::Camera::new(self.camera_identifier.as_deref())?;
        let features = get_features(&camera)?.collect::<Vec<_>>();
       
        // let (str, _size) = aravis::DeviceExt::genicam_xml(
        //     &camera
        //         .device()
        //         .ok_or_else(|| anyhow!("Device not available"))?,
        // );
        // std::fs::write("genicam.xml", str.as_bytes())?;
        // let parser = camera
        //     .create_chunk_parser()
        //     .ok_or_else(|| anyhow!("Couldn't create chunk parser"))?;

        // camera.set_chunk_state("Counter0Value", true)?;
        // debug!("ChunkMode: {}", camera.chunk_mode()?);

        if self.is_termination_requested.load(Ordering::Relaxed) {
            return Ok(StreamingAction::Stop);
        }
        // camera.set_access_check_policy(aravis::AccessCheckPolicy::Disable);
        // camera.set_register_cache_policy(aravis::RegisterCachePolicy::Debug);

        // camera.set_string("DeviceScanType", "Areascan")?;
        // camera.set_string("RegionSelector", "Scan3dExtraction1")?;
        // dbg!(camera.dup_available_enumerations_as_display_names("ComponentSelector"));
        // dbg!(camera.boolean("OnlyInScan3dRegionWithReflectance")?);
        // dbg!(camera.is_feature_implemented("Array_Region_IsImplementedSwissKnife_"));

        // camera.set_string("ComponentSelector", "Scatter")?;

        trace!("Create Camera with features {:?}", features);
        (self.callback)(&mut camera)?;
        
        
        camera.set_acquisition_mode(AcquisitionMode::Continuous)?;
        let stream = camera.create_stream()?;

        let size = camera.payload()?;

        let (send_buf_back, mut recv_buffer) = futures::channel::mpsc::channel(10);

        for _ in 0..2 {
            stream.push_buffer(Buffer::new_allocate(size as _));
        }

        camera.start_acquisition()?;

        let mut last = std::time::Instant::now();
        loop {
            trace!("Before pop_buffer");
            if self.is_termination_requested.load(Ordering::Relaxed) {
                break;
            }
            let Some(mut buf) = stream.timeout_pop_buffer(5_100_000) else {
                if let Ok(x) = camera.string("DeviceSerialNumber") {
                    debug!("No images but still able to get serial number {x}. Avoid reconnect. If stream stopped unexpectedly, try to reduce the Bandwidth (FrameRate, enable JumboFrames...): {:?}", StructuredStatistics::new(&stream));
                    continue;
                }
                return Err(anyhow!(
                    "A long time elapsed without images: {:?}",
                    last.elapsed()
                ));
            };
            trace!("After pop_buffer: {:?}", StructuredStatistics::new(&stream));

            let elapsed = last.elapsed();
            last = std::time::Instant::now();
            // let pilatus_image = convert_buf.try_into_pilatus_luma();
            // stream.push_buffer(&buf);
            if buf.status() != BufferStatus::Success {
                debug!("{elapsed:?} invalid buffer received: {:?}", buf.status());
                // returning existing buffer caused segfault
                // stream.push_buffer(Buffer::new_leaked_box(size as _));
                stream.push_buffer(buf);

                if recv_buffer.is_terminated() {
                    break;
                } else {
                    trace!("Continue with stream");
                    continue;
                }
            }

            trace!("PayloadSize: {}", buf.data().1);
            match buf.payload_type() {
                BufferPayloadType::Image | BufferPayloadType::Multipart => {
                    // match parser.string_value(&buf, "Counter0Value") {
                    //     Ok(o) => debug!("Found int value: {o}"),
                    //     Err(e) => error!("Counter not available: {e:?}"),
                    // }

                    //debug!("ChunkData: {:?}", buf.chunk_data(0))
                }
                // BufferPayloadType::ChunkData => {
                //     // for x in 0..100 {
                //     //     let data = buf.chunk_data(x);
                //     //     if data.is_empty() {
                //     //         break;
                //     //     }
                //     //     debug!("Got chunk {x} of len {}: {:?}", data.len(), &data[0..10]);
                //     // }
                //     debug!("Got Chunk, which was discarded");
                //     stream.push_buffer(buf);
                //     continue;
                // }
                t => {
                    trace!(
                        "{elapsed:?} unexpected payload: {t:?} of size {}",
                        buf.data().1
                    );
                    // return existing buf caused deadlock
                    stream.push_buffer(buf);
                    continue;
                }
            }

            let mut convert_buf = match recv_buffer.try_next() {
                Ok(Some(b)) => b,
                _ => {
                    info!("No buffer in pool. Allocate new");
                    Box::new(ReturnableBuffer::new(
                        Buffer::new_allocate(size as _),
                        send_buf_back.clone(),
                    ))
                }
            };
            convert_buf.swap_buf(&mut buf);
            stream.push_buffer(buf);

            let image_buf = convert_buf.buffer();
            let nparts = image_buf.n_parts();
            anyhow::ensure!(nparts > 0, "Expected at least one image part");
            trace!("Got buffer with {nparts} parts");
            // Collect others into vecs, use the buffer only for main-image for now (to be optimized)

            let iter = (0..nparts)
                .filter(|part_id| {
                    let ptype = image_buf.part_data_type(*part_id);
                    match ptype {
                        BufferPartDataType::ChunkData
                        | BufferPartDataType::ConfidenceMap
                        | BufferPartDataType::DeviceSpecific
                        | BufferPartDataType::Jpeg
                        | BufferPartDataType::Jpeg2000
                        | BufferPartDataType::Unknown => {
                            trace!("Ignore {ptype:?}");
                            false
                        }
                        _ => true,
                    }
                })
                .map(|part_id| {
                    try_into_dynamic_pilatus_image(part_id, convert_buf.clone())
                        .map(|image| (part_id, image_buf.part_component_id(part_id), image))
                        .with_context(|| format!("Cannot create part {part_id} of {nparts}"))
                });
            let mut first_component_id = None;
            let component_map = {
                let mut component_map = BTreeMap::new();
                for data in iter {
                    let (part_id, component_id, image) = data?;
                    first_component_id.get_or_insert(component_id);
                    match component_map.entry(component_id) {
                        Entry::Vacant(vacant_entry) => {
                            vacant_entry.insert(vec![(part_id, image)]);
                        }
                        Entry::Occupied(mut occupied_entry) => {
                            occupied_entry.get_mut().push((part_id, image));
                        }
                    }
                }
                component_map
            };
            let mut component_images: HashMap<_, _> = component_map
                .into_iter()
                .filter_map(|(component_id, parts)| {
                    let mut iter = parts.into_iter().fuse();
                    match (iter.next(), iter.next(), iter.next(), iter.next()) {
                        (Some(x), None, None, None) => Some((component_id, x.1)),
                        (Some(r), Some(g), Some(b), None) => {
                            let (width, height) = r.1.dimensions();
                            if r.1.dimensions() != g.1.dimensions() || r.1.dimensions() != b.1.dimensions() {
                                warn!("Got Image with different sizes. Component '{component_id}' will be ignored");
                                return None;
                            }
                            let len = (width.get() * height.get()) as usize;
                            match (r.1, g.1, b.1) {
                                (DynamicImage::Luma8(r), DynamicImage::Luma8(g), DynamicImage::Luma8(b)) => {
                                    assert_eq!(r.buffer().len(),g.buffer().len());
                                    assert_eq!(r.buffer().len(),b.buffer().len());
                                    
                                    let buf = unsafe {
                                        let mut buf: Arc<[MaybeUninit<u8>]> = Arc::new_uninit_slice((len * 3) as usize);
                                        let mut_slice = Arc::get_mut(&mut buf).expect("Just created");
                                        std::ptr::copy_nonoverlapping(r.buffer().as_ptr(), mut_slice[0].as_mut_ptr(), len);
                                        std::ptr::copy_nonoverlapping(g.buffer().as_ptr(), mut_slice[len].as_mut_ptr(), len);
                                        std::ptr::copy_nonoverlapping(b.buffer().as_ptr(), mut_slice[len*2].as_mut_ptr(), len);
                                        buf.assume_init()
                                    };
                                    let img = GenericImage::<u8, 3>::new_arc(buf, width, height);
                                    Some((component_id, DynamicImage::Rgb8Planar(img)))
                                },
                                x => {
                                    warn!("Only 3xLuma8 can be combined. Got {x:?}");
                                    None
                                },
                            }
                            
                            
                        }
                        (a, b, c, d) => {
                            warn!(
                                "Expected either 1 or three, got {} {} {} {}",
                                a.is_some(),
                                b.is_some(),
                                c.is_some(),
                                d.is_some()
                            );
                            None
                        }
                    }
                })
                .collect();

            let Some((_main_conponent_id, pilatus_image)) =
                first_component_id.and_then(|id| component_images.remove(&id).map(|img| (id, img)))
            else {
                warn!("No image found");
                convert_buf.release();
                continue;
            };

            let others = component_images
                .into_iter()
                .map(|(component_id, image)| {
                    anyhow::Ok((
                        SpecificImageKey::try_from(Cow::Owned(format!("component{component_id}")))
                            .unwrap(),
                        image,
                    ))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;
            /*
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

            let result = ImageWithMeta::with_meta_and_others(
                pilatus_image,
                ImageMeta { hash: None },
                others,
            );

            match (callback)(result) {
                StreamingAction::Continue => {}
                StreamingAction::Stop => return Ok(StreamingAction::Stop),
            }
        }

        debug!("Running out of buffers?");
        Ok(StreamingAction::Stop)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct StructuredStatistics {
    n_completed_buffers: u64,
    n_failures: u64,
    n_underruns: u64,
}

impl StructuredStatistics {
    fn new(stream: &aravis::Stream) -> Self {
        let (n_completed_buffers, n_failures, n_underruns) = stream.statistics();
        Self {
            n_completed_buffers,
            n_failures,
            n_underruns,
        }
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
                .run(|img| {
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
        while let Some((
            i,
            ImageWithMeta {
                image: pilatus_engineering::image::DynamicImage::Luma8(img),
                ..
            },
        )) = stream.next().await
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
