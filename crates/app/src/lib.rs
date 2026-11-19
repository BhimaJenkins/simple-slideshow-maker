//! Simple Slideshow Maker: the egui front end.

mod app;
mod audio;
mod export_ui;
mod inspector;
mod theme;
mod timeline;
mod worker;

pub use app::SlideshowApp;

pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Simple Slideshow Maker")
            .with_inner_size([1360.0, 840.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    }
}

/// Android entry point (built with `cargo apk` / `xbuild`).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: egui_winit::winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let mut options = native_options();
    options.android_app = Some(app);
    let _ = eframe::run_native(
        "Simple Slideshow Maker",
        options,
        Box::new(|cc| Ok(Box::new(SlideshowApp::new(cc)))),
    );
}
