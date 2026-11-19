//! The export dialog and the background export job.

use crate::app::SlideshowApp;
use crate::theme;
use egui::RichText;
use slideshow_core::export::{self, ExportFormat, ExportSettings, Quality};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

pub struct ExportJob {
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: Receiver<Result<PathBuf, String>>,
}

pub struct ExportUi {
    pub open: bool,
    pub settings: ExportSettings,
    pub job: Option<ExportJob>,
    pub result: Option<Result<PathBuf, String>>,
    use_target: bool,
    target_mb: f64,
}

impl Default for ExportUi {
    fn default() -> Self {
        Self {
            open: false,
            settings: ExportSettings::for_format(ExportFormat::Mp4),
            job: None,
            result: None,
            use_target: false,
            target_mb: 25.0,
        }
    }
}

fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 { format!("{mb:.1} MB") } else { format!("{:.0} KB", bytes as f64 / 1024.0) }
}

impl SlideshowApp {
    pub(crate) fn export_window(&mut self, ctx: &egui::Context) {
        // Finish a running job.
        if let Some(job) = &self.export.job {
            if let Ok(res) = job.done.try_recv() {
                self.export.result = Some(res);
                self.export.job = None;
                self.export.open = true;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if !self.export.open {
            return;
        }
        let mut open = true;
        let mut start: Option<PathBuf> = None;
        egui::Window::new(RichText::new("Export").strong())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(380.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let ex = &mut self.export;
                if let Some(job) = &ex.job {
                    let p = job.progress.load(Ordering::Relaxed) as f32 / 1000.0;
                    ui.label("Exporting…");
                    ui.add(egui::ProgressBar::new(p).show_percentage().animate(true));
                    if ui.button("Cancel").clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                if let Some(res) = &ex.result {
                    match res {
                        Ok(path) => ui.label(RichText::new(format!("✔ Saved to {}", path.display())).color(egui::Color32::from_rgb(120, 220, 150))),
                        Err(e) => ui.label(RichText::new(format!("✖ {e}")).color(egui::Color32::from_rgb(255, 120, 120))),
                    };
                    ui.separator();
                }

                theme::section(ui, "Format");
                let mut fmt = ex.settings.format;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut fmt, ExportFormat::Mp4, ExportFormat::Mp4.label());
                    ui.selectable_value(&mut fmt, ExportFormat::Gif, ExportFormat::Gif.label());
                });
                if fmt != ex.settings.format {
                    ex.settings = ExportSettings::for_format(fmt);
                }
                let s = &mut ex.settings;

                theme::section(ui, "Size");
                let heights: &[u32] = match s.format {
                    ExportFormat::Mp4 => &[2160, 1440, 1080, 720, 480, 360],
                    ExportFormat::Gif => &[720, 480, 360, 270, 240],
                };
                ui.horizontal_wrapped(|ui| {
                    for &h in heights {
                        ui.selectable_value(&mut s.height, h, format!("{h}p"));
                    }
                });
                let fps_opts: &[u32] = match s.format {
                    ExportFormat::Mp4 => &[24, 25, 30, 60],
                    ExportFormat::Gif => &[8, 10, 12, 15, 20, 25],
                };
                ui.horizontal(|ui| {
                    ui.label("Frame rate");
                    for &f in fps_opts {
                        ui.selectable_value(&mut s.fps, f, format!("{f}"));
                    }
                });

                theme::section(ui, "Quality");
                ui.horizontal(|ui| {
                    for q in Quality::ALL {
                        ui.selectable_value(&mut s.quality, q, q.label());
                    }
                });
                if s.format == ExportFormat::Mp4 {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut ex.use_target, "Aim for a file size of");
                        ui.add_enabled(ex.use_target, egui::DragValue::new(&mut ex.target_mb).range(1.0..=4000.0).speed(0.5).suffix(" MB"));
                    });
                    s.target_mb = ex.use_target.then_some(ex.target_mb);
                } else {
                    ui.label(RichText::new("Tip: smaller size and fewer frames per second make much smaller GIFs.").size(11.0).color(theme::TEXT_DIM));
                }

                ui.separator();
                let (w, h) = s.size(&self.project);
                ui.label(RichText::new(format!(
                    "{w}×{h} · {:.1} s · about {}",
                    self.project.duration(),
                    human_size(s.estimate_bytes(&self.project))
                )).color(theme::TEXT_DIM));
                ui.add_space(4.0);
                if theme::primary_button(ui, "Export…").clicked() {
                    start = pick_output(s.format);
                }
            });
        self.export.open = open;
        if let Some(path) = start {
            self.start_export(path);
        }
    }

    fn start_export(&mut self, path: PathBuf) {
        let project = self.project.clone();
        let settings = self.export.settings.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (p2, c2) = (progress.clone(), cancel.clone());
        std::thread::spawn(move || {
            let res = export::export(&project, &settings, &path, &mut |f| {
                p2.store((f * 1000.0) as u32, Ordering::Relaxed);
                !c2.load(Ordering::Relaxed)
            });
            let _ = tx.send(res.map_err(|e| e.to_string()));
        });
        self.export.result = None;
        self.export.job = Some(ExportJob { progress, cancel, done: rx });
    }
}

fn pick_output(format: ExportFormat) -> Option<PathBuf> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let ext = format.ext();
        rfd::FileDialog::new()
            .add_filter(ext.to_uppercase(), &[ext])
            .set_file_name(format!("My slideshow.{ext}"))
            .save_file()
            .map(|p| if p.extension().is_none() { p.with_extension(ext) } else { p })
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // Mobile: write into the app's documents folder.
        let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        Some(dir.join(format!("slideshow.{}", format.ext())))
    }
}
