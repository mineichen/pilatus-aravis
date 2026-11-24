use std::num::NonZeroU32;

use aravis::{
    glib::translate::ToGlibPtr,
    BufferPayloadType,
};
use imbuf::{
    DynamicImageChannel, DynamicImageChannelView, ImageChannel, ImageChannelVTable, PixelType,
    PixelTypePrimitive,
};
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
    #[error("Unsupported PixelFormat {format}: {bits_per_pixel} bits per pixel is not a multiple of 8 (e.g. packed formats)")]
    UnsupportedBitDepth { format: String, bits_per_pixel: u8 },}

pub(crate) trait ToImbufImageExt {
    fn try_into_pilatus<T: PixelType + Clone>(
        self,
    ) -> Result<ImageChannel<T::Primitive>, ToPilatusImageError>;
}

// Workaround inability to have static which uses Outer Generics
trait Factory<T: 'static> {
    const VTABLE: &'static ImageChannelVTable;
}

unsafe extern "C-unwind" fn clone_returnable<'a>(
    image: &DynamicImageChannelView<'a>,
) -> DynamicImageChannelView<'a> {
    let cloned_ptr =
        unsafe { ReturnableBuffer::clone_raw(DynamicImageChannelView::raw_data(image)) };
    unsafe {
        DynamicImageChannelView::from_raw_parts(
            DynamicImageChannelView::raw_ptr(image),
            image.width(),
            image.height(),
            image.pixel_elements(),
            DynamicImageChannelView::raw_vtable(image),
            cloned_ptr,
        )
    }
}

/// Whether the buffer of the channel is also referenced by a clone of it
unsafe fn is_shared(image: &DynamicImageChannelView<'_>) -> bool {
    !unsafe { ReturnableBuffer::is_unique_raw(DynamicImageChannelView::raw_data(image)) }
}

unsafe extern "C-unwind" fn make_mut<T: PixelTypePrimitive>(
    image: &mut DynamicImageChannelView<'_>,
) {
    if unsafe { is_shared(image) } {
        let (width, height, pixel_elements) =
            (image.width(), image.height(), image.pixel_elements());
        // Assigning drops the previous channel, which releases its share of the buffer.
        let copy = unsafe { image.buffer_flat_unchecked::<T>() }.to_vec();
        *image = unsafe {
            DynamicImageChannel::new_vec_flat_unchecked(copy, width, height, pixel_elements)
        };
    }
}

/// `make_mut` without copying: The buffer can only be mutated if no clone shares it
unsafe extern "C-unwind" fn try_make_mut(image: &mut DynamicImageChannelView<'_>) -> bool {
    !unsafe { is_shared(image) }
}

unsafe extern "C-unwind" fn clear(image: &mut DynamicImageChannelView<'_>) {
    // The last clone returns the buffer to the pool
    drop(unsafe { ReturnableBuffer::from_raw(DynamicImageChannelView::raw_data(image)) });
}

impl<T: PixelTypePrimitive> Factory<T> for ReturnableBuffer {
    const VTABLE: &'static ImageChannelVTable =
        &ImageChannelVTable::new::<T>(clone_returnable, make_mut::<T>, try_make_mut, clear);
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

pub(crate) fn try_into_dynamic_pilatus_image_channel(
    part_id: u32,
    buffer: ReturnableBuffer,
) -> Result<DynamicImageChannel, ToPilatusImageError> {
    let image_buf = buffer.buffer();
    let format = image_buf.part_pixel_format(part_id);
    // TODO: Support bit depths which aren't a multiple of 8 once imbuf supports them
    let bits_per_pixel = aravis::bits_per_pixel(format);
    if bits_per_pixel % 8 != 0 {
        return Err(ToPilatusImageError::UnsupportedBitDepth {
            format: format!("{format:?}"),
            bits_per_pixel,
        });
    }

    match bits_per_pixel / 8 {
        1 => (part_id, buffer).try_into_pilatus::<u8>().map(Into::into),
        2 => (part_id, buffer).try_into_pilatus::<u16>().map(Into::into),
        byte_depth => Err(ToPilatusImageError::InvalidPixelType {
            details: format!("Unsupported byte depth {byte_depth} of PixelFormat {format:?}"),
        }),
    }
}

/// Buffer should be reusable without reallocation to vec
impl ToImbufImageExt for (u32, ReturnableBuffer) {
    fn try_into_pilatus<T: PixelType + Clone>(
        self,
    ) -> Result<ImageChannel<T::Primitive>, ToPilatusImageError> {
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
        let (x, len) = part_data(buffer, idx);
        let width = buffer.part_width(idx) as u32;
        let height = buffer.part_height(idx) as u32;

        assert!(
            (x as *mut T).is_aligned(),
            "aravis returned misaligned image data"
        );
        let (non_zero_width, non_zero_height) = check_dimensions::<T>(width, height, len)?;
        Ok(unsafe {
            ImageChannel::<T::Primitive>::new_with_vtable(
                x as _,
                non_zero_width,
                non_zero_height,
                <ReturnableBuffer as Factory<T::Primitive>>::VTABLE,
                returnable_buffer.into_raw(),
            )
        })
    }
}

fn check_dimensions<T>(
    width: u32,
    height: u32,
    buf_size: usize,
) -> Result<(NonZeroU32, NonZeroU32), ToPilatusImageError> {
    let invalid_size = || ToPilatusImageError::InvalidSize { width, height };
    let non_zero_width = NonZeroU32::new(width).ok_or_else(invalid_size)?;
    let non_zero_height = NonZeroU32::new(height).ok_or_else(invalid_size)?;

    // Not zero, because width and height aren't
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
    #[cfg_attr(miri, ignore = "uses aravis FFI which Miri does not support")]
    fn channel_borrows_aravis_buffer_and_returns_it_to_pool() {
        // FakeCamera defaults to 512x512 Mono8
        let buffer = aravis::Buffer::new_allocate(512 * 512);
        aravis::FakeCamera::new("serial_number").fill_buffer(&buffer);
        let data_ptr = part_data(&buffer, 0).0.cast_const();
        let (sender, mut pool) = futures::channel::mpsc::channel(10);
        let returnable = ReturnableBuffer::new(buffer, sender);

        let mut image = (0, returnable).try_into_pilatus::<u8>().unwrap();
        assert_eq!(image.buffer().as_ptr(), data_ptr, "conversion must not copy");

        let mut clone = image.clone();
        assert_eq!(clone.buffer().as_ptr(), data_ptr, "clone must not copy");
        assert!(clone.buffer_mut().is_none(), "shared buffer is not mutable");

        let original = image.buffer().to_vec();
        clone.make_mut()[0] = original[0].wrapping_add(1);
        assert_ne!(clone.buffer().as_ptr(), data_ptr, "copy on write");
        assert_eq!(image.buffer(), &original[..]);
        assert!(pool.try_recv().is_err(), "buffer is still in use");

        let unique_ptr = image.buffer_mut().map(|b| b.as_ptr());
        assert_eq!(unique_ptr, Some(data_ptr), "unique buffer is mutable in place");

        drop(image);
        let returned = pool.try_recv().expect("buffer returned to pool");
        assert_eq!(part_data(&returned, 0).0.cast_const(), data_ptr);
    }
}
