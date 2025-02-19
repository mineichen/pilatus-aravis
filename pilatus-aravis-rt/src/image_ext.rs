use std::{num::NonZeroU32, sync::Arc};

use aravis::{
    glib::{object::ObjectExt, translate::ToGlibPtr},
    BufferPayloadType,
};
use pilatus_engineering::image::{DynamicImage, GenericImage, ImageVtable};
use tracing::debug;

use crate::buffer::ReturnableBuffer;

#[derive(Debug, thiserror::Error)]
pub enum ToPilatusImageError {
    #[error("The provided Buffer is not an image")]
    NotAnImage,
    #[error("Unsupported PixelType: {details}")]
    InvalidPixelType { details: String },
    #[error("Conversion failed: width {width}, height: {height}")]
    InvalidSize { width: u32, height: u32 },
}

pub(crate) trait ToPilatusImageExt {
    fn try_into_pilatus<T: Clone, const CHANNELS: usize>(
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
    let buf = unsafe { &*(image.data as *mut ReturnableBuffer) }
        .buffer()
        .ref_count();
    let refcount = buf;
    if refcount == 1 {
        image.ptr as *mut T
    } else {
        let mut arc_image = Arc::from(image.buffer());
        let (width, height) = image.dimensions();
        let ptr = Arc::get_mut(&mut arc_image).unwrap();
        let ptr = (ptr as *mut [T]).cast();

        let mut new_image = GenericImage::<T, CHANNELS>::new_arc(arc_image, width, height);
        std::mem::swap(image, &mut new_image);
        ptr
    }
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
} /*

  impl ToPilatusImageExt for Box<ReturnableBuffer> {
      fn try_into_pilatus<T: AnyBitPattern + NoUninit, const CHANNELS: usize>(
          self,
      ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError> {
          let buffer = self.buffer();
          if !matches!(
              buffer.payload_type(),
              BufferPayloadType::Image | BufferPayloadType::Multipart
          ) {
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
  */

pub(crate) fn try_into_dynamic_pilatus_image(
    part_id: u32,
    buffer: Box<ReturnableBuffer>,
) -> Result<DynamicImage, ToPilatusImageError> {
    let image_buf = buffer.buffer();
    let width = image_buf.part_width(part_id);
    let height = image_buf.part_height(part_id);
    let area = width as usize * height as usize;
    let (_, buffer_size) = part_data(&image_buf, part_id);
    let byte_dept = buffer_size / area;
    debug!("Rest size: {}", buffer_size - area * byte_dept);

    match byte_dept {
        1 => (part_id, buffer)
            .try_into_pilatus()
            .map(pilatus_engineering::image::DynamicImage::Luma8),
        2 => (part_id, buffer)
            .try_into_pilatus()
            .map(pilatus_engineering::image::DynamicImage::Luma16),
        _ => Err(crate::ToPilatusImageError::InvalidPixelType {
            details: format!(
                "Unknown bit dept of PixelFormat {:?}. Area {area}, BufferSize: {buffer_size}",
                image_buf.part_pixel_format(part_id)
            ),
        }),
    }
}

/// Buffer should be reusable without reallocation to vec
impl<'a> ToPilatusImageExt for (u32, Box<ReturnableBuffer>) {
    fn try_into_pilatus<T: Clone, const CHANNELS: usize>(
        self,
    ) -> Result<GenericImage<T, CHANNELS>, ToPilatusImageError> {
        let (idx, returnable_buffer) = self;
        let buffer = returnable_buffer.buffer();
        debug!(
            "Kind: {:?}, Component_ID: {}",
            buffer.part_data_type(idx),
            buffer.part_component_id(idx)
        );
        if !matches!(
            buffer.payload_type(),
            BufferPayloadType::Image | BufferPayloadType::Multipart
        ) {
            return Err(ToPilatusImageError::NotAnImage);
        }
        let (x, len) = part_data(&buffer, idx);
        let width = buffer.part_width(idx) as u32;
        let height = buffer.part_height(idx) as u32;

        assert!((x as *mut T).is_aligned());
        let (non_zero_width, non_zero_height) = check_dimensions::<T>(width, height, len)?;
        let boxed = Box::into_raw(returnable_buffer) as usize;
        Ok(unsafe {
            GenericImage::<T, CHANNELS>::new_with_vtable(
                x as _,
                non_zero_width,
                non_zero_height,
                <Box<ReturnableBuffer> as Factory<T, CHANNELS>>::VTABLE,
                boxed,
            )
        })
    }
}

fn check_dimensions<T>(
    width: u32,
    height: u32,
    buf_size: usize,
) -> Result<(NonZeroU32, NonZeroU32), ToPilatusImageError> {
    let area = height as usize * width as usize;
    let pixel_size = buf_size / area;

    if pixel_size != std::mem::size_of::<T>() {
        return Err(ToPilatusImageError::InvalidPixelType {
            details: {
                let expected_dept = std::mem::size_of::<T>();
                format!("Expected {expected_dept} byte deep (area: {area}, size: {buf_size})")
            },
        });
    }

    let non_zero_width = NonZeroU32::try_from(width)
        .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;
    let non_zero_height = NonZeroU32::try_from(height)
        .map_err(|_| ToPilatusImageError::InvalidSize { width, height })?;
    Ok((non_zero_width, non_zero_height))
}

pub fn part_data(buf: &aravis::Buffer, part_id: u32) -> (*mut u8, usize) {
    unsafe {
        let mut size = 0usize;
        let data = aravis_sys::arv_buffer_get_part_data(
            buf.to_glib_none().0,
            part_id,
            &mut size as *mut usize,
        );
        (data, size)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn miri_get_mut_on_cloned() {
        let buffer = aravis::Buffer::new_allocate(10000);
        aravis::FakeCamera::new("serial_number").fill_buffer(&buffer);
        let (sender, recv) = futures::channel::mpsc::channel(10);
        let returnable = Box::new(ReturnableBuffer::new(buffer, sender));
        //let image = (0, returnable).try_into_pilatus::<u8, 1>().unwrap();
    }
}
