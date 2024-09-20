use std::{num::NonZeroU32, sync::Arc};

use aravis::BufferPayloadType;
use futures::channel::mpsc::Sender;
use pilatus_engineering::image::{GenericImage, ImageVtable};

#[derive(Debug, thiserror::Error)]
pub enum ToPilatusImageError {
    #[error("The provided Buffer is not an image")]
    NotAnImage,
    #[error("{format:?} is not {expected_dept} bit deep (area: {area}, size: {buffer_size}")]
    InvalidPixelType {
        expected_dept: u8,
        buffer_size: usize,
        area: usize,
        format: aravis::PixelFormat,
    },
    #[error("Conversion failed: width {width}, height: {height}")]
    InvalidSize { width: u32, height: u32 },
}

pub(crate) trait ToPilatusImageExt {
    fn try_into_pilatus<T: Clone, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError>;
}

pub(crate) struct ReturnableBuffer {
    buf: aravis::Buffer,
    ch: Sender<Box<Self>>,
}

impl ReturnableBuffer {
    pub fn new(buf: aravis::Buffer, ch: Sender<Box<Self>>) -> Self {
        Self { buf, ch }
    }
    pub fn swap_buf(&mut self, other: &mut aravis::Buffer) {
        std::mem::swap(&mut self.buf, other);
    }
}

// Workaroung inability to have static which uses Outer Generics
trait Factory<T: 'static, const CHANNELS: usize> {
    const VTABLE: &'static ImageVtable<T, CHANNELS>;
}

extern "C" fn clone_arc<T: Clone, const CHANNELS: usize>(
    image: &GenericImage<T, CHANNELS>,
) -> GenericImage<T, CHANNELS> {
    let (width, height) = image.dimensions();
    GenericImage::new_arc(Arc::from(image.buffer()), width, height)
}

unsafe extern "C" fn make_mut<T: Clone, const CHANNELS: usize>(
    image: &mut GenericImage<T, CHANNELS>,
    out_len: &mut usize,
) -> *mut T {
    *out_len = image.len();
    image.ptr as *mut T
}

impl<T: 'static + Clone, const CHANNELS: usize> Factory<T, CHANNELS> for Box<ReturnableBuffer> {
    const VTABLE: &'static ImageVtable<T, CHANNELS> = {
        extern "C" fn clear<T, const CHANNELS: usize>(img: &mut GenericImage<T, CHANNELS>) {
            let boxed = unsafe { Box::from_raw(img.data as *mut ReturnableBuffer) };
            boxed.ch.clone().try_send(boxed).ok();
        }

        &ImageVtable {
            make_mut,
            drop: clear,
            clone: clone_arc,
        }
    };
}

impl ToPilatusImageExt for Box<ReturnableBuffer> {
    fn try_into_pilatus<T: Clone, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError> {
        if self.buf.payload_type() != BufferPayloadType::Image {
            return Err(ToPilatusImageError::NotAnImage);
        }
        let (buf, len) = self.buf.data();
        let buf = buf as *const T;
        let width = self.buf.image_width() as u32;
        let height = self.buf.image_height() as u32;
        let area = height as usize * width as usize;
        let pixel_size = len / area;
        let remainer = len % area;

        if remainer != 0 || pixel_size != std::mem::size_of::<T>() {
            let pixel_format = self.buf.image_pixel_format();
            return Err(ToPilatusImageError::InvalidPixelType {
                expected_dept: std::mem::size_of::<T>() as _,
                format: pixel_format,
                buffer_size: len,
                area,
            });
        }

        if self.buf.image_pixel_format() == aravis::PixelFormat::MONO_8 {}

        let non_zero_width = NonZeroU32::try_from(width)
            .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;
        let non_zero_height = NonZeroU32::try_from(height)
            .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;

        let boxed = Box::into_raw(self) as usize;
        Ok(unsafe {
            GenericImage::<T, CHANNELS>::new_with_vtable(
                buf,
                non_zero_width,
                non_zero_height,
                <Self as Factory<T, CHANNELS>>::VTABLE,
                boxed,
            )
        })
    }
}
