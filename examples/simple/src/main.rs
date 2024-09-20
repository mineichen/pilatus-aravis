// cargo run --example simple --release
// visit "http://localhost/api/image/viewer"

use minfac::ServiceCollection;
use pilatus_rt::Runtime;

fn main() {
    Runtime::default()
        .register(pilatus_engineering::register)
        .register(pilatus_axum_rt::register)
        .register(pilatus_aravis::register)
        .register(register)
        .run();
}

extern "C" fn register(c: &mut ServiceCollection) {
    c.register(|| {
        pilatus::InitRecipeListener::new(|r| {
            r.add_device(pilatus_aravis::create_default_camera_config());
        })
    });
}
