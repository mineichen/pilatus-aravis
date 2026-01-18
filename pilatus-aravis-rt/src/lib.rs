use minfac::ServiceCollection;

mod buffer;
mod device;
mod genicam;
mod image_ext;
mod ip;
mod wrapper;

pub use image_ext::*;

pub extern "C" fn register(c: &mut ServiceCollection) {
    device::register_services(c);
    ip::register_services(c);
    wrapper::register_services(c);
}

pub use device::create_default_camera_config;
