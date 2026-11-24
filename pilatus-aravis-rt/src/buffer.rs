use std::{mem::ManuallyDrop, sync::Arc};

use futures::channel::mpsc::Sender;

/// Shared ownership of a filled aravis buffer.
///
/// Clones share the buffer. When the last clone is dropped, the buffer is sent back to the pool.
/// `Arc` decides which drop is the last one atomically, so a buffer is never lost when clones
/// are dropped concurrently on different threads.
#[derive(Clone)]
pub(crate) struct ReturnableBuffer(Arc<PooledBuffer>);

struct PooledBuffer {
    buf: ManuallyDrop<aravis::Buffer>,
    pool: Sender<aravis::Buffer>,
}

// SAFETY: `aravis::Buffer` is `Send` but not `Sync`. Shared access only calls read-only getters
// (data pointer, part sizes and formats) on a buffer the stream no longer writes to. The
// buffer is only used mutably in `drop`, when no other reference exists.
unsafe impl Sync for PooledBuffer {}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        // SAFETY: `buf` isn't accessed after this, and `drop` runs only once
        let buf = unsafe { ManuallyDrop::take(&mut self.buf) };
        // If the pool is full or closed, the buffer is dropped instead
        self.pool.try_send(buf).ok();
    }
}

impl ReturnableBuffer {
    pub fn new(buf: aravis::Buffer, pool: Sender<aravis::Buffer>) -> Self {
        Self(Arc::new(PooledBuffer {
            buf: ManuallyDrop::new(buf),
            pool,
        }))
    }

    pub fn buffer(&self) -> &aravis::Buffer {
        &self.0.buf
    }

    /// Whether no other clone shares the buffer.
    ///
    /// Reliable without races: if this is the only clone, no other thread can create a new one.
    pub fn is_unique(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }

    /// Turns this clone into a raw pointer (e.g. as the data of an imbuf vtable), without
    /// allocating. Release it with [`Self::from_raw`].
    pub fn into_raw(self) -> *mut () {
        Arc::into_raw(self.0).cast_mut().cast()
    }

    /// Takes back ownership of a clone created by [`Self::into_raw`].
    ///
    /// # Safety
    /// `ptr` must come from [`Self::into_raw`] and must not be released already.
    pub unsafe fn from_raw(ptr: *mut ()) -> Self {
        Self(unsafe { Arc::from_raw(ptr.cast_const().cast::<PooledBuffer>()) })
    }

    /// Creates another raw clone, by only increasing the reference count.
    ///
    /// # Safety
    /// Same as [`Self::from_raw`]. `ptr` stays valid and the returned clone must be released
    /// separately.
    pub unsafe fn clone_raw(ptr: *mut ()) -> *mut () {
        unsafe { Arc::increment_strong_count(ptr.cast_const().cast::<PooledBuffer>()) };
        ptr
    }

    /// [`Self::is_unique`] for a raw clone, without taking ownership.
    ///
    /// # Safety
    /// Same as [`Self::from_raw`].
    pub unsafe fn is_unique_raw(ptr: *mut ()) -> bool {
        ManuallyDrop::new(unsafe { Self::from_raw(ptr) }).is_unique()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "uses aravis FFI which Miri does not support")]
    fn concurrently_dropped_clones_return_buffer_exactly_once() {
        for _ in 0..100 {
            let (sender, mut pool) = futures::channel::mpsc::channel(10);
            let returnable = ReturnableBuffer::new(aravis::Buffer::new_allocate(16), sender);
            let barrier = std::sync::Barrier::new(4);
            std::thread::scope(|s| {
                for clone in std::iter::repeat_n(returnable, 4) {
                    let barrier = &barrier;
                    s.spawn(move || {
                        barrier.wait();
                        drop(clone);
                    });
                }
            });
            assert!(pool.try_recv().is_ok(), "buffer returned to pool");
            assert!(pool.try_recv().is_err(), "buffer returned only once");
        }
    }
}
