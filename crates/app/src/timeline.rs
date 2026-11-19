//! The timeline: media, text and music tracks with a ruler and playhead.

use crate::app::{fmt_time, paint_cover, BinPayload, DropTarget, Selection, SlideshowApp};
use crate::theme;
use egui::{pos2, vec2, Color32, FontId, Id, Rect, RichText, Sense, Stroke};
use slideshow_core::{media, timing, MediaKind, TransitionKind};

const GUTTER: f32 = 78.0;
const RULER_H: f32 = 22.0;
const MEDIA_H: f32 = 64.0;
const TEXT_H: f32 = 32.0;
const MUSIC_H: f32 = 40.0;
const GAP: f32 = 5.0;
const EDGE: f32 = 7.0;
const LEFT_PAD: f32 = 8.0;

/// Geometry from the last frame, used to place files dropped from the OS.
#[derive(Clone, Copy)]
struct Geometry {
    media: Rect,
    music: Rect,
    origin_x: f32,
    zoom: f32,
}

fn geom_id() -> Id {
    Id::unique("timeline-geometry")
}

/// Where a drop at the current pointer position should go.
pub fn drop_target_at_pointer(ctx: &egui::Context, app: &SlideshowApp) -> DropTarget {
    let Some(g) = ctx.data(|d| d.get_temp::<Geometry>(geom_id())) else { return DropTarget::End };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return DropTarget::End };
    target_for(app, g, pos)
}

fn target_for(app: &SlideshowApp, g: Geometry, pos: egui::Pos2) -> DropTarget {
    let t = ((pos.x - g.origin_x) / g.zoom).max(0.0) as f64;
    if g.media.contains(pos) {
        DropTarget::MediaIndex(media_index_at(app, t))
    } else if g.music.contains(pos) {
        DropTarget::MusicAt(t)
    } else {
        DropTarget::End
    }
}

/// Index a clip dropped at time `t` should be inserted at.
fn media_index_at(app: &SlideshowApp, t: f64) -> usize {
    let starts = app.project.clip_starts();
    starts.iter().zip(&app.project.clips).filter(|(s, c)| **s + c.duration / 2.0 < t).count()
}

impl SlideshowApp {
    pub(crate) fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let dur = self.project.duration();
        ui.horizontal(|ui| {
            theme::section(ui, "Timeline");
            ui.label(RichText::new(format!("{} · {} clips", fmt_time(dur), self.project.clips.len())).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Fit").on_hover_text("Zoom to show the whole slideshow").clicked() {
                    let w = (ui.available_width() + 200.0 - GUTTER - 60.0).max(100.0);
                    self.zoom = (w / dur.max(1.0) as f32).clamp(4.0, 400.0);
                }
                ui.add(egui::Slider::new(&mut self.zoom, 4.0..=400.0).logarithmic(true).show_value(false));
                ui.label(RichText::new("Zoom").color(theme::TEXT_DIM));
            });
        });
        ui.add_space(2.0);

        let total_h = RULER_H + MEDIA_H + TEXT_H + MUSIC_H + GAP * 3.0;
        ui.horizontal_top(|ui| {
            // Track labels.
            let (gut, _) = ui.allocate_exact_size(vec2(GUTTER, total_h), Sense::hover());
            let p = ui.painter();
            let mut y = gut.min.y + RULER_H + GAP;
            for (name, h, color) in [
                ("Media", MEDIA_H, theme::CLIP_MEDIA),
                ("Text", TEXT_H, theme::CLIP_TEXT),
                ("Music", MUSIC_H, theme::CLIP_MUSIC),
            ] {
                let r = Rect::from_min_size(pos2(gut.min.x, y), vec2(GUTTER - 6.0, h));
                p.rect_filled(r, 5, theme::BG_RAISED);
                p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, h)), 2, color);
                p.text(r.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, name, FontId::proportional(12.5), theme::TEXT);
                y += h + GAP;
            }

            egui::ScrollArea::horizontal().id_salt("timeline-scroll").auto_shrink(false).show(ui, |ui| {
                self.tracks(ui, total_h);
            });
        });
    }

    fn tracks(&mut self, ui: &mut egui::Ui, total_h: f32) {
        let dur = self.project.duration();
        let zoom = self.zoom;
        let width = (ui.available_width()).max((dur as f32 + 8.0) * zoom + LEFT_PAD * 2.0);
        let (area, bg) = ui.allocate_exact_size(vec2(width, total_h), Sense::click_and_drag());
        let ox = area.min.x + LEFT_PAD;
        let x_of = |t: f64| ox + t as f32 * zoom;
        let t_of = |x: f32| ((x - ox) / zoom).max(0.0) as f64;

        let ruler = Rect::from_min_size(area.min, vec2(width, RULER_H));
        let media_r = Rect::from_min_size(pos2(area.min.x, ruler.max.y + GAP), vec2(width, MEDIA_H));
        let text_r = Rect::from_min_size(pos2(area.min.x, media_r.max.y + GAP), vec2(width, TEXT_H));
        let music_r = Rect::from_min_size(pos2(area.min.x, text_r.max.y + GAP), vec2(width, MUSIC_H));
        ui.ctx().data_mut(|d| d.insert_temp(geom_id(), Geometry { media: media_r, music: music_r, origin_x: ox, zoom }));

        let painter = ui.painter_at(area);
        for r in [media_r, text_r, music_r] {
            painter.rect_filled(r, 5, theme::BG_TRACK);
        }
        if let Some(limit) = self.project.time_limit {
            let x = x_of(limit);
            painter.line_segment([pos2(x, ruler.min.y), pos2(x, area.max.y)], Stroke::new(1.0, theme::ACCENT.linear_multiply(0.5)));
        }

        // Ruler ticks.
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]
            .into_iter()
            .find(|s| *s as f32 * zoom >= 60.0)
            .unwrap_or(600.0);
        let mut t = 0.0;
        let end_t = t_of(area.max.x);
        while t <= end_t {
            let x = x_of(t);
            painter.line_segment([pos2(x, ruler.max.y - 7.0), pos2(x, ruler.max.y)], Stroke::new(1.0, theme::TEXT_DIM));
            painter.text(pos2(x + 3.0, ruler.min.y + 2.0), egui::Align2::LEFT_TOP, fmt_time(t), FontId::monospace(10.0), theme::TEXT_DIM);
            let minor = step / 5.0;
            for k in 1..5 {
                let xm = x_of(t + minor * k as f64);
                painter.line_segment([pos2(xm, ruler.max.y - 3.0), pos2(xm, ruler.max.y)], Stroke::new(1.0, theme::BORDER));
            }
            t += step;
        }

        // Clicking or dragging on the ruler / empty space moves the playhead.
        if bg.clicked() || bg.dragged() {
            if let Some(pos) = bg.interact_pointer_pos() {
                self.seek(t_of(pos.x));
                if bg.clicked() && !ruler.contains(pos) {
                    self.selection = Selection::None;
                }
            }
        }

        self.media_track(ui, &painter, media_r, ox);
        self.text_track(ui, &painter, text_r, ox);
        self.music_track(ui, &painter, music_r, ox);

        // Drops from the media bin.
        if let Some(payload) = egui::DragAndDrop::payload::<BinPayload>(ui.ctx()) {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                let geom = Geometry { media: media_r, music: music_r, origin_x: ox, zoom };
                let kind = media::kind_of(&payload.0);
                // Show where it will land.
                if media_r.contains(pos) && kind != Some(MediaKind::Audio) {
                    let idx = media_index_at(self, t_of(pos.x));
                    let x = x_of(self.project.clip_starts().get(idx).copied().unwrap_or(dur));
                    painter.line_segment([pos2(x, media_r.min.y), pos2(x, media_r.max.y)], Stroke::new(3.0, theme::ACCENT));
                } else if music_r.contains(pos) && kind == Some(MediaKind::Audio) {
                    painter.rect_stroke(music_r, 5, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
                }
                if ui.input(|i| i.pointer.any_released()) && area.contains(pos) {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    let target = match (target_for(self, geom, pos), kind) {
                        (DropTarget::MusicAt(_), Some(k)) if k != MediaKind::Audio => DropTarget::End,
                        (DropTarget::MediaIndex(_), Some(MediaKind::Audio)) => DropTarget::End,
                        (t, _) => t,
                    };
                    self.import(vec![payload.0.clone()], true, target);
                }
            }
        }

        // Playhead.
        let x = x_of(self.playhead);
        painter.line_segment([pos2(x, ruler.min.y), pos2(x, area.max.y)], Stroke::new(1.5, theme::PLAYHEAD));
        let head = vec![pos2(x - 6.0, ruler.min.y), pos2(x + 6.0, ruler.min.y), pos2(x, ruler.min.y + 9.0)];
        painter.add(egui::Shape::convex_polygon(head, theme::PLAYHEAD, Stroke::NONE));

        // Keep the playhead in view while playing.
        if self.playing {
            ui.scroll_to_rect(Rect::from_center_size(pos2(x, area.center().y), vec2(120.0, 10.0)), None);
        }
    }

    fn media_track(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, track: Rect, ox: f32) {
        let zoom = self.zoom;
        let starts = self.project.clip_starts();
        let mut reorder: Option<(usize, usize)> = None;
        let mut resized = false;

        for i in 0..self.project.clips.len() {
            let (id, dur, kind, start) = {
                let c = &self.project.clips[i];
                (c.id, c.duration, c.kind, starts[i])
            };
            let r = Rect::from_min_size(pos2(ox + start as f32 * zoom, track.min.y), vec2((dur as f32 * zoom).max(4.0), track.height()));
            let body = ui.interact(r.shrink2(vec2(EDGE / 2.0, 0.0)), Id::unique(("clip", id)), Sense::click_and_drag());
            let edge_r = Rect::from_min_max(pos2(r.max.x - EDGE, r.min.y), pos2(r.max.x + 2.0, r.max.y));
            let edge = ui.interact(edge_r, Id::unique(("clip-edge", id)), Sense::drag());
            let selected = self.selection == Selection::Clip(id);

            if body.clicked() || body.drag_started() {
                self.selection = Selection::Clip(id);
            }
            let mut draw_r = r;
            if body.dragged() {
                draw_r = r.translate(vec2(body.total_drag_delta().map_or(0.0, |d| d.x), 0.0));
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let to = media_index_at(self, ((pos.x - ox) / zoom) as f64);
                    let x = ox + self.project.clip_starts().get(to).copied().unwrap_or(self.project.duration()) as f32 * zoom;
                    painter.line_segment([pos2(x, track.min.y), pos2(x, track.max.y)], Stroke::new(3.0, theme::ACCENT));
                }
            }
            if body.drag_stopped() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let to = media_index_at(self, ((pos.x - ox) / zoom) as f64);
                    reorder = Some((i, to));
                }
            }
            if edge.hovered() || edge.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if edge.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let c = &mut self.project.clips[i];
                    let mut d = (((pos.x - r.min.x) / zoom) as f64).max(0.3);
                    if let (MediaKind::Video, Some(l)) = (c.kind, c.source_length) {
                        d = d.min((l - c.source_in).max(0.3));
                    }
                    c.duration = (d * 10.0).round() / 10.0;
                    if self.project.time_limit.is_some() {
                        c.locked_duration = true;
                    }
                }
            }
            if edge.drag_stopped() {
                resized = true;
            }

            // Draw the clip: thumbnail strip with a name label.
            let fill = if selected { theme::CLIP_MEDIA.gamma_multiply(1.25) } else { theme::CLIP_MEDIA };
            painter.rect_filled(draw_r, 5, fill);
            let inner = draw_r.shrink(3.0);
            if let Some(tex) = self.thumbs.get(&self.project.clips[i].path) {
                let tw = (inner.height() * 16.0 / 9.0).min(inner.width());
                let mut x = inner.min.x;
                let clip_painter = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
                while x < inner.max.x {
                    paint_cover(&clip_painter, tex, Rect::from_min_size(pos2(x, inner.min.y), vec2(tw, inner.height())));
                    x += tw + 2.0;
                    if tw < 8.0 {
                        break;
                    }
                }
                clip_painter.rect_filled(Rect::from_min_max(pos2(inner.min.x, inner.max.y - 16.0), inner.max), 0, Color32::from_black_alpha(150));
            }
            let c = &self.project.clips[i];
            let icon = if kind == MediaKind::Video { "🎞 " } else { "" };
            let lock = if c.locked_duration && self.project.time_limit.is_some() { " 🔒" } else { "" };
            let label = format!("{icon}{}  {:.1}s{lock}", c.name(), c.duration);
            painter.with_clip_rect(inner.intersect(painter.clip_rect())).text(
                pos2(inner.min.x + 4.0, inner.max.y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                FontId::proportional(11.0),
                Color32::WHITE,
            );
            let stroke = if selected { Stroke::new(2.0, Color32::WHITE) } else { Stroke::new(1.0, Color32::from_black_alpha(120)) };
            painter.rect_stroke(draw_r, 5, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(Rect::from_min_max(pos2(draw_r.max.x - 4.0, draw_r.min.y + 18.0), pos2(draw_r.max.x - 2.0, draw_r.max.y - 18.0)), 1, Color32::from_white_alpha(90));
        }

        // Transition markers between clips.
        for i in 1..self.project.clips.len() {
            let tr = self.project.clips[i].transition_in;
            if tr.kind == TransitionKind::Cut {
                continue;
            }
            let x = ox + starts[i] as f32 * zoom;
            let w = (timing::transition_len(&self.project, i) as f32 * zoom).max(12.0);
            let r = Rect::from_center_size(pos2(x, track.min.y + 11.0), vec2(w, 14.0));
            let color = match tr.kind {
                TransitionKind::DipToWhite => Color32::from_white_alpha(220),
                TransitionKind::DipToBlack => Color32::from_rgb(20, 20, 20),
                _ => Color32::from_white_alpha(120),
            };
            painter.rect_filled(r, 7, color);
            painter.rect_stroke(r, 7, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Inside);
            let resp = ui.interact(r, Id::unique(("transition", self.project.clips[i].id)), Sense::click());
            if resp.clicked() {
                self.selection = Selection::Clip(self.project.clips[i].id);
            }
            resp.on_hover_text(format!("{} · {:.1}s (click to edit)", tr.kind.label(), tr.duration));
        }

        if let Some((from, mut to)) = reorder {
            if to > from {
                to -= 1;
            }
            if to != from {
                let c = self.project.clips.remove(from);
                self.project.clips.insert(to.min(self.project.clips.len()), c);
            }
        }
        if resized || reorder.is_some() {
            timing::auto_fit(&mut self.project);
        }
    }

    fn text_track(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, track: Rect, ox: f32) {
        let zoom = self.zoom;
        let total = self.project.duration();
        for i in 0..self.project.texts.len() {
            let t = &self.project.texts[i];
            let id = t.id;
            let r = Rect::from_min_size(pos2(ox + t.start as f32 * zoom, track.min.y + 3.0), vec2((t.duration as f32 * zoom).max(6.0), track.height() - 6.0));
            let body = ui.interact(r.shrink2(vec2(EDGE / 2.0, 0.0)), Id::unique(("text", id)), Sense::click_and_drag());
            let edge = ui.interact(Rect::from_min_max(pos2(r.max.x - EDGE, r.min.y), pos2(r.max.x + 2.0, r.max.y)), Id::unique(("text-edge", id)), Sense::drag());
            if body.clicked() || body.drag_started() {
                self.selection = Selection::Text(id);
            }
            let t = &mut self.project.texts[i];
            if body.dragged() {
                t.start = (t.start + (body.drag_delta().x / zoom) as f64).clamp(0.0, (total - t.duration).max(0.0));
            }
            if edge.hovered() || edge.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if edge.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    t.duration = (((pos.x - r.min.x) / zoom) as f64).clamp(0.3, (total - t.start).max(0.3));
                }
            }
            let selected = self.selection == Selection::Text(id);
            let r = Rect::from_min_size(pos2(ox + t.start as f32 * zoom, r.min.y), vec2((t.duration as f32 * zoom).max(6.0), r.height()));
            painter.rect_filled(r, 5, if selected { theme::CLIP_TEXT.gamma_multiply(1.25) } else { theme::CLIP_TEXT });
            if selected {
                painter.rect_stroke(r, 5, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
            let label = format!("T  {}", t.text.lines().next().unwrap_or(""));
            painter.with_clip_rect(r.shrink(2.0).intersect(painter.clip_rect())).text(
                r.left_center() + vec2(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::proportional(11.5),
                Color32::WHITE,
            );
        }
    }

    fn music_track(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, track: Rect, ox: f32) {
        let zoom = self.zoom;
        let mut changed = false;
        for i in 0..self.project.music.len() {
            let m = &self.project.music[i];
            let id = m.id;
            let r = Rect::from_min_size(pos2(ox + m.start as f32 * zoom, track.min.y + 3.0), vec2((m.duration as f32 * zoom).max(6.0), track.height() - 6.0));
            let body = ui.interact(r.shrink2(vec2(EDGE / 2.0, 0.0)), Id::unique(("music", id)), Sense::click_and_drag());
            let edge = ui.interact(Rect::from_min_max(pos2(r.max.x - EDGE, r.min.y), pos2(r.max.x + 2.0, r.max.y)), Id::unique(("music-edge", id)), Sense::drag());
            if body.clicked() || body.drag_started() {
                self.selection = Selection::Music(id);
            }
            let m = &mut self.project.music[i];
            if body.dragged() {
                m.start = (m.start + (body.drag_delta().x / zoom) as f64).max(0.0);
            }
            if edge.hovered() || edge.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if edge.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    let max = m.source_length.map(|l| l - m.source_in).unwrap_or(f64::MAX);
                    m.duration = (((pos.x - r.min.x) / zoom) as f64).clamp(0.3, max.max(0.3));
                }
            }
            changed |= body.drag_stopped();
            let selected = self.selection == Selection::Music(id);
            let r = Rect::from_min_size(pos2(ox + m.start as f32 * zoom, r.min.y), vec2((m.duration as f32 * zoom).max(6.0), r.height()));
            painter.rect_filled(r, 5, if selected { theme::CLIP_MUSIC.gamma_multiply(1.25) } else { theme::CLIP_MUSIC });
            // A simple decorative waveform so the track reads as audio.
            let clip_p = painter.with_clip_rect(r.shrink(2.0).intersect(painter.clip_rect()));
            let mid = r.center().y;
            let mut x = r.min.x + 4.0;
            let mut k = (id as f32) * 1.7;
            while x < r.max.x - 2.0 {
                k += 0.9;
                let a = ((k.sin() * 0.5 + (k * 2.3).sin() * 0.3 + (k * 0.37).sin() * 0.2).abs() * 0.8 + 0.15) * (r.height() / 2.0 - 4.0);
                clip_p.line_segment([pos2(x, mid - a), pos2(x, mid + a)], Stroke::new(1.5, Color32::from_white_alpha(70)));
                x += 4.0;
            }
            if selected {
                painter.rect_stroke(r, 5, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
            clip_p.text(r.left_top() + vec2(6.0, 2.0), egui::Align2::LEFT_TOP, format!("♫ {}", m.name()), FontId::proportional(11.0), Color32::WHITE);
        }
        if changed {
            timing::fit_music(&mut self.project);
        }
    }
}
