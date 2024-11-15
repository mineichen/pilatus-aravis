use std::{
    num::{NonZero, NonZeroU32},
    sync::Arc,
};

use aravis::{BufferPayloadType, PixelFormat};
use bytemuck::{pod_collect_to_vec, AnyBitPattern, NoUninit};
use pilatus_engineering::image::{GenericImage, ImageVtable};

use crate::buffer::ReturnableBuffer;

#[derive(Debug, thiserror::Error)]
pub enum ToPilatusImageError {
    #[error("The provided Buffer is not an image")]
    NotAnImage,
    #[error("Unsupported PixelType {format:?}: {details}")]
    InvalidPixelType {
        format: aravis::PixelFormat,
        details: String,
    },
    #[error("Conversion failed: width {width}, height: {height}")]
    InvalidSize { width: u32, height: u32 },
}

pub(crate) trait ToPilatusImageExt {
    fn try_into_pilatus<T: AnyBitPattern + NoUninit, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError>;
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
            unsafe { Box::from_raw(img.data as *mut ReturnableBuffer) }.release();
        }

        &ImageVtable {
            make_mut,
            drop: clear,
            clone: clone_arc,
        }
    };
}

impl ToPilatusImageExt for Box<ReturnableBuffer> {
    fn try_into_pilatus<T: AnyBitPattern + NoUninit, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError> {
        let buffer = self.buffer();
        if buffer.payload_type() != BufferPayloadType::Image {
            return Err(ToPilatusImageError::NotAnImage);
        }
        let (buf, len) = buffer.data();
        let buf = buf as *const T;

        let width = buffer.image_width() as u32;
        let height = buffer.image_height() as u32;

        let (non_zero_width, non_zero_height) =
            check_dimensions::<T>(buffer.image_pixel_format(), width, height, len)?;

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

/// Buffer should be reusable without reallocation to vec
impl<'a> ToPilatusImageExt for (u32, &'a aravis::Buffer) {
    fn try_into_pilatus<T: AnyBitPattern + NoUninit, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError> {
        let (idx, buffer) = self;
        if buffer.payload_type() != BufferPayloadType::Image {
            return Err(ToPilatusImageError::NotAnImage);
        }

        let pixel_buffer_typed: Vec<T> = bytemuck::try_cast_vec::<_, T>(buffer.part_data(idx))
            .unwrap_or_else(|(_, original)| pod_collect_to_vec(&original));
        let len = pixel_buffer_typed.len();

        let width = buffer.part_width(idx) as u32;
        let height = buffer.part_height(idx) as u32;

        let (non_zero_width, non_zero_height) =
            check_dimensions::<T>(buffer.part_pixel_format(idx), width, height, len)?;

        Ok(GenericImage::<T, CHANNELS>::new_vec(
            pixel_buffer_typed,
            non_zero_width,
            non_zero_height,
        ))
    }
}

fn check_dimensions<T>(
    pixel_format: PixelFormat,
    width: u32,
    height: u32,
    buf_size: usize,
) -> Result<(NonZeroU32, NonZeroU32), ToPilatusImageError> {
    let area = height as usize * width as usize;
    let pixel_size = buf_size / area;
    let remainer = buf_size % area;

    if remainer != 0 || pixel_size != std::mem::size_of::<T>() {
        return Err(ToPilatusImageError::InvalidPixelType {
            format: pixel_format,
            details: {
                let area = area;
                let buffer_size = buf_size;
                let expected_dept = std::mem::size_of::<T>();
                format!("Expected {expected_dept} bit deep (area: {area}, size: {buffer_size})")
            },
        });
    }

    let non_zero_width = NonZeroU32::try_from(width)
        .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;
    let non_zero_height = NonZeroU32::try_from(height)
        .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;
    Ok((non_zero_width, non_zero_height))
}
