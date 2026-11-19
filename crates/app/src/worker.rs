//! Background threads: the preview renderer and the thumbnail loader, so the
//! interface never waits on decoding.

use slideshow_core::{media, render::Renderer, MediaKind, Project};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub struct FrameRequest {
    pub project: Arc<Project>,
    pub t: f64,
    pub size: [u32; 2],
    pub video_fps: f64,
    pub playing: bool,
}

pub struct RenderedFrame {
    pub image: egui::ColorImage,
    pub t: f64,
    pub errors: Vec<String>,
}

/// Renders preview frames off the UI thread. Only the newest request is drawn.
pub struct PreviewWorker {
    tx: Sender<FrameRequest>,
    rx: Receiver<RenderedFrame>,
    pub in_flight: bool,
}

impl PreviewWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<FrameRequest>();
        let (res_tx, res_rx) = channel();
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                // Scan system fonts here rather than on the UI thread.
                let _ = slideshow_core::text::fonts();
                let mut r = Renderer::new();
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    r.video_fps = req.video_fps;
                    r.skip_edge_fades = !req.playing;
                    r.retain_for(&req.project);
                    let img = r.render(&req.project, req.t, req.size[0], req.size[1]);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width() as usize, img.height() as usize],
                        img.as_raw(),
                    );
                    let errors = std::mem::take(&mut r.errors);
                    if res_tx.send(RenderedFrame { image, t: req.t, errors }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn preview thread");
        Self { tx: req_tx, rx: res_rx, in_flight: false }
    }

    pub fn request(&mut self, req: FrameRequest) {
        if self.tx.send(req).is_ok() {
            self.in_flight = true;
        }
    }

    pub fn poll(&mut self) -> Option<RenderedFrame> {
        let mut last = None;
        while let Ok(f) = self.rx.try_recv() {
            last = Some(f);
            self.in_flight = false;
        }
        last
    }
}

pub struct Thumb {
    pub path: PathBuf,
    pub image: Option<egui::ColorImage>,
}

/// Loads small thumbnails for the media bin and timeline.
pub struct ThumbWorker {
    tx: Sender<(PathBuf, MediaKind)>,
    rx: Receiver<Thumb>,
}

impl ThumbWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<(PathBuf, MediaKind)>();
        let (res_tx, res_rx) = channel();
        std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                while let Ok((path, kind)) = req_rx.recv() {
                    let image = media::thumbnail(&path, kind, 160).map(|img| {
                        egui::ColorImage::from_rgba_unmultiplied(
                            [img.width() as usize, img.height() as usize],
                            img.as_raw(),
                        )
                    });
                    if res_tx.send(Thumb { path, image }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn thumbnail thread");
        Self { tx: req_tx, rx: res_rx }
    }

    pub fn request(&self, path: PathBuf, kind: MediaKind) {
        let _ = self.tx.send((path, kind));
    }

    pub fn poll(&self) -> Vec<Thumb> {
        self.rx.try_iter().collect()
    }
}
