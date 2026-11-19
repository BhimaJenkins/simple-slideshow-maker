//! The right-hand panel: settings for whatever is selected, or the project.

use crate::app::{fmt_time, Selection, SlideshowApp};
use crate::theme;
use egui::{RichText, Slider};
use slideshow_core::*;

fn combo<T: Copy + PartialEq>(ui: &mut egui::Ui, id: &str, value: &mut T, all: &[T], label: fn(T) -> &'static str) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(170.0).selected_text(label(*value)).show_ui(ui, |ui| {
        for &v in all {
            changed |= ui.selectable_value(value, v, label(v)).changed();
        }
    });
    changed
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([86.0, 20.0], egui::Label::new(RichText::new(label).color(theme::TEXT_DIM)));
        add(ui);
    });
}

fn motion_ui(ui: &mut egui::Ui, id: &str, m: &mut Motion) {
    row(ui, "Animation", |ui| {
        combo(ui, &format!("{id}-motion"), &mut m.kind, &MotionKind::ALL, MotionKind::label);
    });
    if m.kind != MotionKind::None {
        row(ui, "Intensity", |ui| {
            ui.add(Slider::new(&mut m.intensity, 0.0..=1.0).show_value(false));
        });
        row(ui, "Speed", |ui| {
            ui.add(Slider::new(&mut m.speed, 0.25..=3.0).logarithmic(true).suffix("×").fixed_decimals(2));
        });
    }
    if m.kind == MotionKind::KenBurns {
        row(ui, "Pan", |ui| {
            combo(ui, &format!("{id}-pan"), &mut m.direction, &PanDirection::ALL, PanDirection::label);
        });
    }
}

fn transition_ui(ui: &mut egui::Ui, id: &str, t: &mut Transition) {
    row(ui, "Type", |ui| {
        combo(ui, &format!("{id}-tr"), &mut t.kind, &TransitionKind::ALL, TransitionKind::label);
    });
    if t.kind != TransitionKind::Cut {
        row(ui, "Length", |ui| {
            ui.add(Slider::new(&mut t.duration, 0.1..=4.0).suffix(" s").fixed_decimals(1));
        });
    }
}

impl SlideshowApp {
    pub(crate) fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        match self.selection {
            Selection::Clip(id) => self.clip_inspector(ui, id),
            Selection::Text(id) => self.text_inspector(ui, id),
            Selection::Music(id) => self.music_inspector(ui, id),
            Selection::None => self.project_inspector(ui),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("Back to project settings").clicked() {
                    self.selection = Selection::None;
                }
                if ui.small_button("🗑").on_hover_text("Remove (Delete)").clicked() {
                    self.delete_selection();
                }
            });
        });
        ui.separator();
    }

    fn clip_inspector(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(idx) = self.project.clips.iter().position(|c| c.id == id) else {
            self.selection = Selection::None;
            return;
        };
        let name = self.project.clips[idx].name();
        self.header(ui, &name);
        let limited = self.project.time_limit.is_some();
        let mut refit = false;
        let mut apply_motion = None;
        let mut apply_transition = None;
        {
            let c = &mut self.project.clips[idx];
            theme::section(ui, "Timing");
            row(ui, "Duration", |ui| {
                let max = match (c.kind, c.source_length) {
                    (MediaKind::Video, Some(l)) => (l - c.source_in).max(0.1),
                    _ => 600.0,
                };
                if ui.add(egui::DragValue::new(&mut c.duration).range(0.3..=max).speed(0.05).suffix(" s").max_decimals(2)).changed() && limited {
                    c.locked_duration = true;
                    refit = true;
                }
            });
            if limited {
                row(ui, "", |ui| {
                    if ui.checkbox(&mut c.locked_duration, "Keep this length when fitting").changed() {
                        refit = true;
                    }
                });
            }
            if c.kind == MediaKind::Video {
                row(ui, "Start at", |ui| {
                    let max = c.source_length.unwrap_or(3600.0);
                    ui.add(egui::DragValue::new(&mut c.source_in).range(0.0..=max).speed(0.05).suffix(" s"));
                });
                if let Some(l) = c.source_length {
                    row(ui, "Video length", |ui| {
                        ui.label(fmt_time(l));
                    });
                }
            }
            row(ui, "Framing", |ui| {
                ui.selectable_value(&mut c.fit, Fit::Fill, "Fill frame");
                ui.selectable_value(&mut c.fit, Fit::Fit, "Show all");
            });

            theme::section(ui, "Frame animation");
            motion_ui(ui, &format!("clip{id}"), &mut c.motion);
            if ui.small_button("Apply to all clips").clicked() {
                apply_motion = Some(c.motion);
            }

            if idx > 0 {
                theme::section(ui, "Transition in");
                transition_ui(ui, &format!("clip{id}"), &mut c.transition_in);
                if ui.small_button("Apply to all clips").clicked() {
                    apply_transition = Some(c.transition_in);
                }
            } else {
                theme::section(ui, "Transition in");
                ui.label(RichText::new("The first clip uses the project's “Fade in at start” setting.").color(theme::TEXT_DIM).size(11.5));
            }
        }
        if let Some(m) = apply_motion {
            for c in self.project.clips.iter_mut().filter(|c| c.kind == MediaKind::Image) {
                c.motion = m;
            }
            self.project.default_motion = m;
        }
        if let Some(t) = apply_transition {
            for c in &mut self.project.clips {
                c.transition_in = t;
            }
            self.project.default_transition = t;
        }
        if refit {
            timing::auto_fit(&mut self.project);
        }
    }

    fn text_inspector(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(idx) = self.project.texts.iter().position(|t| t.id == id) else {
            self.selection = Selection::None;
            return;
        };
        self.header(ui, "Text");
        let total = self.project.duration();
        let families = slideshow_core::text::fonts().families();
        let filter = &mut self.font_filter;
        let t = &mut self.project.texts[idx];

        ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(3).desired_width(f32::INFINITY).hint_text("Type your title…"));
        ui.label(RichText::new("Drag the title in the preview to move it.").size(11.0).color(theme::TEXT_DIM));

        theme::section(ui, "Style");
        row(ui, "Font", |ui| {
            egui::ComboBox::from_id_salt("font").width(170.0).selected_text(&t.font).height(320.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(filter).hint_text("Search fonts"));
                let f = filter.to_lowercase();
                for fam in families.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)).take(400) {
                    ui.selectable_value(&mut t.font, fam.clone(), fam);
                }
            });
        });
        row(ui, "Size", |ui| {
            ui.add(Slider::new(&mut t.size, 0.02..=0.4).show_value(false));
        });
        row(ui, "Color", |ui| {
            ui.color_edit_button_srgba_unmultiplied(&mut t.color);
            ui.checkbox(&mut t.shadow, "Shadow");
        });
        row(ui, "Align", |ui| {
            ui.selectable_value(&mut t.align, TextAlign::Left, "Left");
            ui.selectable_value(&mut t.align, TextAlign::Center, "Center");
            ui.selectable_value(&mut t.align, TextAlign::Right, "Right");
        });
        row(ui, "Position", |ui| {
            if ui.small_button("Top").clicked() {
                t.pos = [0.5, 0.15];
            }
            if ui.small_button("Middle").clicked() {
                t.pos = [0.5, 0.5];
            }
            if ui.small_button("Bottom").clicked() {
                t.pos = [0.5, 0.85];
            }
        });

        theme::section(ui, "Timing");
        row(ui, "Starts at", |ui| {
            ui.add(egui::DragValue::new(&mut t.start).range(0.0..=total.max(0.0)).speed(0.05).suffix(" s").max_decimals(2));
        });
        row(ui, "Duration", |ui| {
            ui.add(egui::DragValue::new(&mut t.duration).range(0.3..=total.max(0.3)).speed(0.05).suffix(" s").max_decimals(2));
        });

        theme::section(ui, "Animation");
        row(ui, "In", |ui| {
            combo(ui, "anim-in", &mut t.anim_in, &TextAnim::ALL, TextAnim::label);
        });
        row(ui, "Out", |ui| {
            combo(ui, "anim-out", &mut t.anim_out, &TextAnim::ALL, TextAnim::label);
        });
        row(ui, "Speed", |ui| {
            ui.add(Slider::new(&mut t.anim_duration, 0.1..=3.0).suffix(" s").fixed_decimals(1));
        });
    }

    fn music_inspector(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(idx) = self.project.music.iter().position(|m| m.id == id) else {
            self.selection = Selection::None;
            return;
        };
        let name = self.project.music[idx].name();
        self.header(ui, &format!("♫ {name}"));
        let m = &mut self.project.music[idx];
        theme::section(ui, "Timing");
        row(ui, "Starts at", |ui| {
            ui.add(egui::DragValue::new(&mut m.start).range(0.0..=36000.0).speed(0.05).suffix(" s").max_decimals(2));
        });
        row(ui, "Duration", |ui| {
            let max = m.source_length.map(|l| l - m.source_in).unwrap_or(36000.0).max(0.3);
            ui.add(egui::DragValue::new(&mut m.duration).range(0.3..=max).speed(0.05).suffix(" s").max_decimals(2));
        });
        row(ui, "Song from", |ui| {
            let max = m.source_length.unwrap_or(36000.0);
            ui.add(egui::DragValue::new(&mut m.source_in).range(0.0..=max).speed(0.05).suffix(" s"));
        });
        theme::section(ui, "Sound");
        row(ui, "Volume", |ui| {
            ui.add(Slider::new(&mut m.volume, 0.0..=2.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        row(ui, "Fade in", |ui| {
            ui.add(Slider::new(&mut m.fade_in, 0.0..=10.0).suffix(" s").fixed_decimals(1));
        });
        row(ui, "Fade out", |ui| {
            ui.add(Slider::new(&mut m.fade_out, 0.0..=10.0).suffix(" s").fixed_decimals(1));
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Music plays in MP4 exports. GIFs have no sound.").size(11.0).color(theme::TEXT_DIM));
    }

    fn project_inspector(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Project").strong().size(15.0));
        ui.separator();
        let p = &mut self.project;

        theme::section(ui, "Format");
        const PRESETS: [(&str, u32, u32); 6] = [
            ("Widescreen 1080p (16:9)", 1920, 1080),
            ("Widescreen 720p (16:9)", 1280, 720),
            ("4K (16:9)", 3840, 2160),
            ("Square (1:1)", 1080, 1080),
            ("Vertical / Stories (9:16)", 1080, 1920),
            ("Classic (4:3)", 1440, 1080),
        ];
        let current = PRESETS
            .iter()
            .find(|(_, w, h)| *w == p.width && *h == p.height)
            .map(|x| x.0.to_string())
            .unwrap_or_else(|| format!("{}×{}", p.width, p.height));
        row(ui, "Size", |ui| {
            egui::ComboBox::from_id_salt("preset").width(170.0).selected_text(current).show_ui(ui, |ui| {
                for (name, w, h) in PRESETS {
                    if ui.selectable_label(p.width == w && p.height == h, name).clicked() {
                        p.width = w;
                        p.height = h;
                    }
                }
            });
        });
        row(ui, "Frame rate", |ui| {
            for f in [24, 30, 60] {
                ui.selectable_value(&mut p.fps, f, format!("{f}"));
            }
        });
        row(ui, "Background", |ui| {
            ui.color_edit_button_srgb(&mut p.background);
        });

        theme::section(ui, "Playback");
        row(ui, "When done", |ui| {
            ui.selectable_value(&mut p.playback, PlaybackMode::Loop, "Loop");
            ui.selectable_value(&mut p.playback, PlaybackMode::Once, "Play once");
        });
        row(ui, "Fades", |ui| {
            ui.vertical(|ui| {
                ui.checkbox(&mut p.fade_in_start, "Fade in at start");
                ui.checkbox(&mut p.fade_out_end, "Fade out at end");
            });
        });

        theme::section(ui, "Length");
        let mut refit = false;
        let mut limited = p.time_limit.is_some();
        row(ui, "Time limit", |ui| {
            if ui.checkbox(&mut limited, "Fit everything into").changed() {
                p.time_limit = limited.then(|| p.duration().max(10.0).round());
                refit = true;
            }
        });
        if let Some(mut l) = p.time_limit {
            row(ui, "", |ui| {
                if ui.add(egui::DragValue::new(&mut l).range(1.0..=7200.0).speed(0.5).suffix(" seconds")).changed() {
                    p.time_limit = Some(l);
                    refit = true;
                }
            });
            ui.label(RichText::new("New pictures are spaced evenly to fill this time. Lock a clip to keep its length.").size(11.0).color(theme::TEXT_DIM));
        } else {
            row(ui, "Each picture", |ui| {
                ui.add(egui::DragValue::new(&mut p.default_duration).range(0.5..=60.0).speed(0.05).suffix(" s"));
            });
            if ui.small_button("Set every picture to this length").clicked() {
                let d = p.default_duration;
                for c in p.clips.iter_mut().filter(|c| c.kind == MediaKind::Image) {
                    c.duration = d;
                }
                refit = true;
            }
        }
        row(ui, "Music", |ui| {
            if ui.checkbox(&mut p.fit_music, "End music with the slideshow").changed() {
                refit = true;
            }
        });

        theme::section(ui, "New clips get");
        motion_ui(ui, "default", &mut p.default_motion);
        transition_ui(ui, "default", &mut p.default_transition);
        if ui.small_button("Apply these to all clips").clicked() {
            let (m, t) = (p.default_motion, p.default_transition);
            for c in &mut p.clips {
                if c.kind == MediaKind::Image {
                    c.motion = m;
                }
                c.transition_in = t;
            }
        }
        if refit {
            timing::auto_fit(p);
        }

        ui.add_space(12.0);
        if media::ffmpeg().is_none() {
            ui.label(
                RichText::new("⚠ FFmpeg not found: pictures and GIF export work, but video clips and MP4 export need FFmpeg installed.")
                    .size(11.5)
                    .color(egui::Color32::from_rgb(255, 190, 80)),
            );
        }
    }
}
