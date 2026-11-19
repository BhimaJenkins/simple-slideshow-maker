//! Application state and the top-level layout.

use crate::audio::AudioPlayer;
use crate::export_ui::ExportUi;
use crate::theme;
use crate::worker::{FrameRequest, PreviewWorker, ThumbWorker};
use egui::{Color32, RichText, TextureHandle, TextureOptions};
use slideshow_core::{media, timing, AudioClip, Clip, MediaKind, PlaybackMode, Project, TextClip};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    None,
    Clip(u64),
    Text(u64),
    Music(u64),
}

#[derive(Clone)]
pub struct BinItem {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub length: Option<f64>,
}

/// Drag-and-drop payload when dragging from the media bin.
#[derive(Clone)]
pub struct BinPayload(pub PathBuf);

/// Where a dropped file should land on the timeline.
#[derive(Clone, Copy)]
pub enum DropTarget {
    /// Append to the end of the matching track.
    End,
    /// Insert into the media track before this clip index.
    MediaIndex(usize),
    /// Place on the music track at this time.
    MusicAt(f64),
}

pub struct SlideshowApp {
    pub project: Project,
    pub project_path: Option<PathBuf>,
    pub selection: Selection,
    pub playhead: f64,
    pub playing: bool,
    last_tick: Option<Instant>,
    pub bin: Vec<BinItem>,
    pub thumbs: HashMap<PathBuf, TextureHandle>,
    thumb_worker: ThumbWorker,
    preview: PreviewWorker,
    preview_tex: Option<TextureHandle>,
    sent: Option<(Project, f64, [u32; 2], bool)>,
    audio: AudioPlayer,
    /// Timeline zoom in pixels per second.
    pub zoom: f32,
    pub export: ExportUi,
    status: Option<(String, Instant)>,
    undo: Vec<Project>,
    redo: Vec<Project>,
    committed: Project,
    pub font_filter: String,
    pub preview_errors: Vec<String>,
}

const AUTOSAVE_KEY: &str = "autosave_project";

impl SlideshowApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        egui_extras_install(&cc.egui_ctx);
        let project: Project = cc
            .storage
            .and_then(|s| s.get_string(AUTOSAVE_KEY))
            .and_then(|s| Project::from_json(&s).ok())
            .unwrap_or_default();
        let mut app = Self {
            committed: project.clone(),
            project,
            project_path: None,
            selection: Selection::None,
            playhead: 0.0,
            playing: false,
            last_tick: None,
            bin: vec![],
            thumbs: HashMap::new(),
            thumb_worker: ThumbWorker::new(cc.egui_ctx.clone()),
            preview: PreviewWorker::new(cc.egui_ctx.clone()),
            preview_tex: None,
            sent: None,
            audio: AudioPlayer::default(),
            zoom: 40.0,
            export: ExportUi::default(),
            status: None,
            undo: vec![],
            redo: vec![],
            font_filter: String::new(),
            preview_errors: vec![],
        };
        app.rebuild_bin_from_project();
        app
    }

    /// Open files passed on the command line (or via "Open with…").
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        if let Some(p) = paths.iter().find(|p| p.extension().is_some_and(|e| e == "ssm")) {
            let p = p.clone();
            self.load_project_from(&p);
        } else if !paths.is_empty() {
            self.new_project();
            self.import(paths, true, DropTarget::End);
        }
    }

    fn rebuild_bin_from_project(&mut self) {
        let items: Vec<BinItem> = self
            .project
            .clips
            .iter()
            .map(|c| BinItem { path: c.path.clone(), kind: c.kind, length: c.source_length })
            .chain(self.project.music.iter().map(|m| BinItem {
                path: m.path.clone(),
                kind: MediaKind::Audio,
                length: m.source_length,
            }))
            .collect();
        self.bin.clear();
        for it in items {
            self.add_to_bin(it);
        }
    }

    fn add_to_bin(&mut self, item: BinItem) {
        if self.bin.iter().any(|b| b.path == item.path) {
            return;
        }
        if item.kind != MediaKind::Audio {
            self.thumb_worker.request(item.path.clone(), item.kind);
        }
        self.bin.push(item);
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// Add files to the bin and (optionally) the timeline.
    pub fn import(&mut self, paths: Vec<PathBuf>, to_timeline: bool, target: DropTarget) {
        let mut skipped = vec![];
        let mut insert_at = match target {
            DropTarget::MediaIndex(i) => i.min(self.project.clips.len()),
            _ => self.project.clips.len(),
        };
        for path in paths {
            let Some(kind) = media::kind_of(&path) else {
                skipped.push(path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                continue;
            };
            let existing = self.bin.iter().find(|b| b.path == path).cloned();
            let item = existing.unwrap_or_else(|| BinItem {
                length: if kind == MediaKind::Image { None } else { media::probe_duration(&path) },
                path: path.clone(),
                kind,
            });
            self.add_to_bin(item.clone());
            if !to_timeline {
                continue;
            }
            match kind {
                MediaKind::Image | MediaKind::Video => {
                    let dur = match (kind, item.length) {
                        (MediaKind::Video, Some(l)) => l,
                        _ => self.project.default_duration,
                    };
                    let mut c = Clip::new(path, kind, dur);
                    c.source_length = item.length;
                    c.motion = self.project.default_motion;
                    if kind == MediaKind::Video {
                        c.motion.kind = slideshow_core::MotionKind::None;
                    }
                    c.transition_in = self.project.default_transition;
                    self.selection = Selection::Clip(c.id);
                    self.project.clips.insert(insert_at, c);
                    insert_at += 1;
                }
                MediaKind::Audio => {
                    let start = match target {
                        DropTarget::MusicAt(t) => t.max(0.0),
                        _ => self.project.music.iter().map(|m| m.end()).fold(0.0, f64::max),
                    };
                    let m = AudioClip::new(path, start, item.length);
                    self.selection = Selection::Music(m.id);
                    self.project.music.push(m);
                }
            }
        }
        if to_timeline {
            timing::auto_fit(&mut self.project);
        }
        if !skipped.is_empty() {
            self.toast(format!("Skipped unsupported files: {}", skipped.join(", ")));
        }
    }

    pub fn pick_and_import(&mut self) {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let mut exts: Vec<&str> = media::IMAGE_EXTS.to_vec();
            exts.extend(media::VIDEO_EXTS);
            exts.extend(media::AUDIO_EXTS);
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("Pictures, video and music", &exts)
                .add_filter("Pictures", media::IMAGE_EXTS)
                .add_filter("Video", media::VIDEO_EXTS)
                .add_filter("Music", media::AUDIO_EXTS)
                .pick_files()
            {
                self.import(files, true, DropTarget::End);
            }
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.toast("File picking on mobile is coming next; see the README.");
    }

    pub fn add_text(&mut self) {
        let mut t = TextClip::new(self.playhead);
        if self.project.duration() > 0.0 {
            t.duration = t.duration.min(self.project.duration());
        }
        self.selection = Selection::Text(t.id);
        self.project.texts.push(t);
        timing::clamp_texts(&mut self.project);
    }

    pub fn delete_selection(&mut self) {
        match self.selection {
            Selection::Clip(id) => {
                self.project.clips.retain(|c| c.id != id);
                timing::auto_fit(&mut self.project);
            }
            Selection::Text(id) => self.project.texts.retain(|t| t.id != id),
            Selection::Music(id) => self.project.music.retain(|m| m.id != id),
            Selection::None => {}
        }
        self.selection = Selection::None;
    }

    pub fn set_playing(&mut self, play: bool) {
        if play && self.playhead >= self.project.duration() - 1e-3 {
            self.playhead = 0.0;
        }
        self.playing = play && self.project.duration() > 0.0;
        self.last_tick = None;
        if self.playing {
            self.audio.play_from(&self.project, self.playhead);
        } else {
            self.audio.stop();
        }
    }

    pub fn seek(&mut self, t: f64) {
        self.playhead = t.clamp(0.0, self.project.duration().max(0.0));
        if self.playing {
            self.audio.play_from(&self.project, self.playhead);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last_tick {
            self.playhead += (now - last).as_secs_f64();
        }
        self.last_tick = Some(now);
        let dur = self.project.duration();
        if self.playhead >= dur {
            match self.project.playback {
                PlaybackMode::Loop => {
                    self.playhead = 0.0;
                    self.audio.play_from(&self.project, 0.0);
                }
                PlaybackMode::Once => {
                    self.playhead = dur;
                    self.set_playing(false);
                }
            }
        }
        ctx.request_repaint();
    }

    // ---- files -------------------------------------------------------------

    pub fn new_project(&mut self) {
        self.set_playing(false);
        self.project = Project::default();
        self.project_path = None;
        self.selection = Selection::None;
        self.playhead = 0.0;
        self.bin.clear();
    }

    pub fn open_project(&mut self) {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let Some(path) = rfd::FileDialog::new().add_filter("Slideshow project", &["ssm"]).pick_file() {
            self.load_project_from(&path);
        }
    }

    pub fn load_project_from(&mut self, path: &Path) {
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| {
            Project::from_json(&s).map_err(|e| e.to_string())
        }) {
            Ok(p) => {
                self.set_playing(false);
                self.project = p;
                self.project_path = Some(path.to_owned());
                self.selection = Selection::None;
                self.playhead = 0.0;
                self.rebuild_bin_from_project();
            }
            Err(e) => self.toast(format!("Could not open project: {e}")),
        }
    }

    pub fn save_project(&mut self, save_as: bool) {
        let path = match (&self.project_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => {
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                {
                    rfd::FileDialog::new()
                        .add_filter("Slideshow project", &["ssm"])
                        .set_file_name("My slideshow.ssm")
                        .save_file()
                }
                #[cfg(any(target_os = "android", target_os = "ios"))]
                None
            }
        };
        let Some(path) = path else { return };
        match self.project.to_json().map_err(|e| e.to_string()).and_then(|s| {
            std::fs::write(&path, s).map_err(|e| e.to_string())
        }) {
            Ok(()) => {
                self.toast(format!("Saved {}", path.display()));
                self.project_path = Some(path);
            }
            Err(e) => self.toast(format!("Could not save: {e}")),
        }
    }

    // ---- undo --------------------------------------------------------------

    fn track_history(&mut self, ctx: &egui::Context) {
        let dragging = ctx.input(|i| i.pointer.any_down());
        if !dragging && self.project != self.committed {
            self.undo.push(std::mem::replace(&mut self.committed, self.project.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.project, prev));
            self.committed = self.project.clone();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.project, next));
            self.committed = self.project.clone();
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let typing = ctx.egui_wants_keyboard_input();
        let (space, del, home, left, right, undo, redo, redo2, save) = ctx.input_mut(|i| {
            (
                !typing && i.consume_key(Modifiers::NONE, Key::Space),
                !typing && (i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)),
                !typing && i.consume_key(Modifiers::NONE, Key::Home),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                !typing && i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        let frame = 1.0 / self.project.fps.max(1) as f64;
        if space {
            self.set_playing(!self.playing);
        }
        if del {
            self.delete_selection();
        }
        if home {
            self.seek(0.0);
        }
        if left {
            self.seek(self.playhead - frame);
        }
        if right {
            self.seek(self.playhead + frame);
        }
        if redo || redo2 {
            self.redo();
        } else if undo {
            self.undo();
        }
        if save {
            self.save_project(false);
        }
    }

    fn handle_os_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if dropped.is_empty() {
            return;
        }
        // A dropped project file opens it; anything else is imported.
        if let Some(p) = dropped.iter().find(|p| p.extension().is_some_and(|e| e == "ssm")) {
            let p = p.clone();
            self.load_project_from(&p);
            return;
        }
        let target = crate::timeline::drop_target_at_pointer(ctx, self);
        self.import(dropped, true, target);
    }

    // ---- panels ------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Simple Slideshow Maker").strong().size(15.0).color(theme::TEXT));
            ui.separator();
            if ui.button("New").clicked() {
                self.new_project();
            }
            if ui.button("Open…").clicked() {
                self.open_project();
            }
            if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                self.save_project(false);
            }
            ui.separator();
            if ui.button("+ Import media").clicked() {
                self.pick_and_import();
            }
            if ui.button("+ Text").clicked() {
                self.add_text();
            }
            ui.separator();
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo")).on_hover_text("Undo (Ctrl+Z)").clicked() {
                self.undo();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo")).on_hover_text("Redo (Ctrl+Shift+Z)").clicked()
            {
                self.redo();
            }
            ui.separator();

            // Time limit: auto-spaces everything to fit.
            let mut limited = self.project.time_limit.is_some();
            if ui
                .checkbox(&mut limited, "Fit to")
                .on_hover_text("Space all pictures evenly so the slideshow lasts exactly this long")
                .changed()
            {
                self.project.time_limit = limited.then(|| self.project.duration().max(10.0).round());
                timing::auto_fit(&mut self.project);
            }
            if let Some(mut limit) = self.project.time_limit {
                let r = ui.add(egui::DragValue::new(&mut limit).range(1.0..=7200.0).speed(0.5).suffix(" s"));
                if r.changed() {
                    self.project.time_limit = Some(limit);
                    timing::auto_fit(&mut self.project);
                }
            } else {
                ui.label(RichText::new("time limit").color(theme::TEXT_DIM));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary_button(ui, "Export").clicked() {
                    self.export.open = true;
                }
                if let Some((msg, at)) = &self.status {
                    if at.elapsed().as_secs_f32() < 5.0 {
                        ui.label(RichText::new(msg).color(theme::TEXT_DIM));
                    }
                }
            });
        });
    }

    fn media_bin(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, "Media");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("+").on_hover_text("Import pictures, video or music").clicked() {
                    self.pick_and_import();
                }
            });
        });
        if self.bin.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Drop pictures, videos\nand music anywhere").color(theme::TEXT_DIM));
            });
            return;
        }
        ui.label(RichText::new("Drag onto the timeline, or double-click to add").size(11.0).color(theme::TEXT_DIM));
        let mut add: Option<PathBuf> = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for item in &self.bin {
                let id = egui::Id::unique(("bin", &item.path));
                let r = ui
                    .dnd_drag_source(id, BinPayload(item.path.clone()), |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 36.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 4, theme::BG_RAISED);
                            if let Some(t) = self.thumbs.get(&item.path) {
                                paint_cover(ui.painter(), t, rect);
                            } else {
                                let icon = match item.kind {
                                    MediaKind::Image => "🖼",
                                    MediaKind::Video => "🎞",
                                    MediaKind::Audio => "♫",
                                };
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon,
                                    egui::FontId::proportional(16.0),
                                    theme::TEXT_DIM,
                                );
                            }
                            ui.vertical(|ui| {
                                let name = item.path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default();
                                ui.add(egui::Label::new(RichText::new(name).size(12.0)).truncate());
                                let sub = match (item.kind, item.length) {
                                    (MediaKind::Image, _) => "Picture".to_string(),
                                    (MediaKind::Video, Some(l)) => format!("Video · {}", fmt_time(l)),
                                    (MediaKind::Video, None) => "Video".into(),
                                    (MediaKind::Audio, Some(l)) => format!("Music · {}", fmt_time(l)),
                                    (MediaKind::Audio, None) => "Music".into(),
                                };
                                ui.label(RichText::new(sub).size(11.0).color(theme::TEXT_DIM));
                            });
                        })
                    })
                    .response;
                if r.double_clicked() {
                    add = Some(item.path.clone());
                }
            }
        });
        if let Some(p) = add {
            self.import(vec![p], true, DropTarget::End);
        }
    }

    fn viewer(&mut self, ui: &mut egui::Ui) {
        let transport_h = 44.0;
        let avail = ui.available_rect_before_wrap();
        let view_area = egui::Rect::from_min_max(avail.min, egui::pos2(avail.max.x, avail.max.y - transport_h));
        ui.painter().rect_filled(view_area, 0, theme::BG_VIEWER);

        // Frame rectangle with the project's aspect ratio.
        let aspect = self.project.aspect();
        let pad = 16.0;
        let max = view_area.shrink(pad);
        let (fw, fh) = if max.width() / max.height().max(1.0) > aspect {
            (max.height() * aspect, max.height())
        } else {
            (max.width(), max.width() / aspect)
        };
        let frame_rect = egui::Rect::from_center_size(max.center(), egui::vec2(fw.max(1.0), fh.max(1.0)));

        // Ask the worker for a frame at this size (in physical pixels, capped for speed).
        let ppp = ui.ctx().pixels_per_point();
        let mut pw = (fw * ppp).min(1280.0);
        let mut ph = pw / aspect;
        if ph > 720.0 {
            ph = 720.0;
            pw = ph * aspect;
        }
        let size = [((pw as u32) / 8 * 8).max(16), ((ph as u32) / 8 * 8).max(16)];
        if let Some(frame) = self.preview.poll() {
            let _ = frame.t;
            match &mut self.preview_tex {
                Some(t) => t.set(frame.image, TextureOptions::LINEAR),
                None => {
                    self.preview_tex = Some(ui.ctx().load_texture("preview", frame.image, TextureOptions::LINEAR))
                }
            }
            for e in frame.errors {
                if !self.preview_errors.contains(&e) {
                    self.preview_errors.push(e);
                }
            }
        }
        let stale = match &self.sent {
            Some((p, t, s, pl)) => {
                p != &self.project || (*t - self.playhead).abs() > 1e-6 || *s != size || *pl != self.playing
            }
            None => true,
        };
        if stale && !self.preview.in_flight {
            self.preview.request(FrameRequest {
                project: Arc::new(self.project.clone()),
                t: self.playhead,
                size,
                video_fps: self.project.fps as f64,
                playing: self.playing,
            });
            self.sent = Some((self.project.clone(), self.playhead, size, self.playing));
        }

        let painter = ui.painter_at(view_area);
        painter.rect_filled(frame_rect, 0, Color32::BLACK);
        if let Some(tex) = &self.preview_tex {
            painter.image(
                tex.id(),
                frame_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.project.clips.is_empty() && self.project.texts.is_empty() {
            painter.text(
                frame_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop pictures, videos and music here\nor use  + Import media",
                egui::FontId::proportional(18.0),
                theme::TEXT_DIM,
            );
        }

        // Text placement: click a title to select it, drag to move it.
        let resp = ui.interact(frame_rect, egui::Id::unique("viewer-frame"), egui::Sense::click_and_drag());
        if resp.clicked() || resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let n = (pos - frame_rect.min) / frame_rect.size();
                let hit = self
                    .project
                    .texts
                    .iter()
                    .filter(|t| self.playhead >= t.start && self.playhead < t.end())
                    .map(|t| (t.id, (t.pos[0] - n.x).abs() + (t.pos[1] - n.y).abs()))
                    .filter(|(_, d)| *d < 0.2)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((id, _)) = hit {
                    self.selection = Selection::Text(id);
                }
            }
        }
        if let Selection::Text(id) = self.selection {
            let fh_px = frame_rect.height();
            let now = self.playhead;
            if let Some(t) = self.project.texts.iter_mut().find(|t| t.id == id && now >= t.start && now < t.end()) {
                if resp.dragged() {
                    let d = resp.drag_delta() / frame_rect.size();
                    t.pos[0] = (t.pos[0] + d.x).clamp(0.0, 1.0);
                    t.pos[1] = (t.pos[1] + d.y).clamp(0.0, 1.0);
                }
                // Approximate box around the selected title.
                let lines = t.text.lines().count().max(1) as f32;
                let cols = t.text.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
                let h = t.size * fh_px * lines * 1.25;
                let w = (t.size * fh_px * cols * 0.55).max(h);
                let c = frame_rect.min + egui::vec2(t.pos[0] * frame_rect.width(), t.pos[1] * frame_rect.height());
                let r = egui::Rect::from_center_size(c, egui::vec2(w, h)).expand(6.0);
                painter.rect_stroke(r, 3, egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Outside);
            }
        }
        if resp.hovered() && matches!(self.selection, Selection::Text(_)) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }

        // Transport bar.
        let bar = egui::Rect::from_min_max(egui::pos2(avail.min.x, view_area.max.y), avail.max);
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(egui::vec2(12.0, 6.0))));
        bar_ui.horizontal_centered(|ui| {
            if ui.button("⏮").on_hover_text("Back to start (Home)").clicked() {
                self.seek(0.0);
            }
            let play_label = if self.playing { "⏸" } else { "▶" };
            if theme::primary_button(ui, play_label).on_hover_text("Play / pause (Space)").clicked() {
                self.set_playing(!self.playing);
            }
            ui.label(
                RichText::new(format!("{}  /  {}", fmt_time(self.playhead), fmt_time(self.project.duration())))
                    .monospace()
                    .color(theme::TEXT),
            );
            ui.separator();
            ui.selectable_value(&mut self.project.playback, PlaybackMode::Loop, "🔁 Loop");
            ui.selectable_value(&mut self.project.playback, PlaybackMode::Once, "Play once");
            if !self.preview_errors.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.label(RichText::new("⚠ Some media could not be shown").color(Color32::from_rgb(255, 190, 80)));
                    r.on_hover_text(self.preview_errors.join("\n"));
                });
            }
        });
        ui.allocate_rect(avail, egui::Sense::hover());
    }
}

/// Draw a thumbnail cropped to fill `rect`.
pub fn paint_cover(p: &egui::Painter, tex: &TextureHandle, rect: egui::Rect) {
    let [w, h] = tex.size();
    let (tw, th) = (w as f32, h as f32);
    let ra = rect.width() / rect.height();
    let ta = tw / th;
    let uv = if ta > ra {
        let f = ra / ta;
        egui::Rect::from_min_max(egui::pos2((1.0 - f) / 2.0, 0.0), egui::pos2((1.0 + f) / 2.0, 1.0))
    } else {
        let f = ta / ra;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - f) / 2.0), egui::pos2(1.0, (1.0 + f) / 2.0))
    };
    p.image(tex.id(), rect, uv, Color32::WHITE);
}

pub fn fmt_time(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{m:02}:{s:04.1}")
}

fn egui_extras_install(_ctx: &egui::Context) {}

impl eframe::App for SlideshowApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        for t in self.thumb_worker.poll() {
            if let Some(img) = t.image {
                let tex = ctx.load_texture(t.path.display().to_string(), img, TextureOptions::LINEAR);
                self.thumbs.insert(t.path, tex);
            }
        }
        self.tick(&ctx);
        self.shortcuts(&ctx);
        self.handle_os_drops(&ctx);

        let panel = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(10, 8));
        egui::Panel::top("toolbar").frame(panel(theme::BG_PANEL)).show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("timeline")
            .resizable(true)
            .default_size(250.0)
            .min_size(170.0)
            .frame(panel(theme::BG_PANEL))
            .show(ui, |ui| self.timeline_ui(ui));
        egui::Panel::left("media")
            .resizable(true)
            .default_size(220.0)
            .min_size(160.0)
            .frame(panel(theme::BG_PANEL))
            .show(ui, |ui| self.media_bin(ui));
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(300.0)
            .min_size(240.0)
            .frame(panel(theme::BG_PANEL))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.inspector_ui(ui));
            });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG_VIEWER)).show(ui, |ui| self.viewer(ui));

        // Hovering files from the OS: show a drop hint.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(screen, 0, Color32::from_black_alpha(140));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to add to your slideshow",
                egui::FontId::proportional(26.0),
                Color32::WHITE,
            );
        }

        self.export_window(&ctx);
        self.track_history(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(s) = self.project.to_json() {
            storage.set_string(AUTOSAVE_KEY, s);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG_APP.to_normalized_gamma_f32()
    }
}
