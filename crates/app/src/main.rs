#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() -> eframe::Result {
    env_logger::init();
    // Files given on the command line (or via "Open with…") are opened on launch.
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
    eframe::run_native(
        "Simple Slideshow Maker",
        simple_slideshow_maker::native_options(),
        Box::new(move |cc| {
            let mut app = simple_slideshow_maker::SlideshowApp::new(cc);
            app.open_paths(paths);
            Ok(Box::new(app))
        }),
    )
}
