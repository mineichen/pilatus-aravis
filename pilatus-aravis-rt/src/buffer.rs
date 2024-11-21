use aravis::glib::object::ObjectExt;
use futures::channel::mpsc::Sender;

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
    pub fn release(self: Box<Self>) {
        // Not atomically correct: Some might be dropped
        if self.buf.ref_count() == 1 {
            self.ch.clone().try_send(self).ok();
        }
    }

    pub fn buffer(&self) -> &aravis::Buffer {
        &self.buf
    }
}
