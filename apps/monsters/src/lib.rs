//! Mossbound Android/native host. Lifecycle and touch live here; rules in the
//! `matterweave-monsters` crate.
pub mod app;
pub mod lifecycle;
pub mod maps;
pub mod visuals;

#[cfg(not(target_os = "android"))]
pub fn run_desktop() {
    use std::path::PathBuf;
    use winit::event_loop::EventLoop;
    let mut save_path = PathBuf::from("mossbound-save.json");
    let mut frame_limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save" => {
                save_path = PathBuf::from(args.next().expect("--save requires a path"));
            }
            "--smoke-frames" => {
                frame_limit = Some(
                    args.next()
                        .expect("--smoke-frames requires a count")
                        .parse()
                        .expect("frame count"),
                );
            }
            other => {
                eprintln!("unknown argument: {other}");
            }
        }
    }
    let event_loop = match EventLoop::new() {
        Ok(loop_) => loop_,
        Err(error) => {
            eprintln!("event loop failed: {error}");
            return;
        }
    };
    let mut app = app::MonsterApp::new(save_path, frame_limit);
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("run failed: {error}");
    }
    if app.failed {
        std::process::exit(1);
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("Mossbound")
            .with_max_level(log::LevelFilter::Info),
    );
    let Some(directory) = android_app.internal_data_path() else {
        log::error!("Android internal storage unavailable");
        return;
    };
    let event_loop = match winit::event_loop::EventLoop::builder()
        .with_android_app(android_app)
        .build()
    {
        Ok(loop_) => loop_,
        Err(error) => {
            log::error!("event loop failed: {error}");
            return;
        }
    };
    let save_path = directory.join("mossbound-save.json");
    let mut app = app::MonsterApp::new(save_path, None);
    if let Err(error) = event_loop.run_app(&mut app) {
        log::error!("run failed: {error}");
    }
}
