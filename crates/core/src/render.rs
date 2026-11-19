//! CPU compositor: turns a project and a time into a picture. The same code
//! drives the live preview (small) and the exporters (full size), so what you
//! see is what you export.

use crate::media::{self, MediaError, VideoDecoder};
use crate::model::*;
use crate::text::{self, TextMask};
use crate::timing::transition_len;
use image::{Rgba, RgbaImage};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Length of the "fade in at start / fade out at end" effect.
pub const EDGE_FADE: f64 = 1.0;

#[derive(Default)]
pub struct Renderer {
    images: HashMap<(PathBuf, u32), Result<Arc<RgbaImage>, String>>,
    videos: HashMap<(u64, u32), VideoDecoder>,
    texts: HashMap<TextKey, Arc<TextMask>>,
    shadows: HashMap<TextKey, Arc<TextMask>>,
    /// Frame rate used when streaming video frames.
    pub video_fps: f64,
    /// Skip the whole-show fade in/out (the paused preview uses this so the
    /// first frame isn't just black).
    pub skip_edge_fades: bool,
    /// Problems hit while rendering (missing files, no FFmpeg...).
    pub errors: Vec<String>,
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct TextKey {
    text: String,
    font: String,
    px: u32,
    align: u8,
}

struct Layer {
    clip: usize,
    local_t: f64,
    opacity: f32,
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Renderer {
    pub fn new() -> Self {
        Self { video_fps: 30.0, ..Default::default() }
    }

    /// Drop cached pictures that are no longer used by the project.
    pub fn retain_for(&mut self, p: &Project) {
        self.images.retain(|(path, _), _| p.clips.iter().any(|c| &c.path == path));
        self.videos.retain(|(id, _), _| p.clips.iter().any(|c| c.id == *id));
        if self.texts.len() > 64 {
            self.texts.clear();
            self.shadows.clear();
        }
    }

    /// Render the frame at `t` seconds at `w`×`h` pixels.
    pub fn render(&mut self, p: &Project, t: f64, w: u32, h: u32) -> RgbaImage {
        let bg = Rgba([p.background[0], p.background[1], p.background[2], 255]);
        let mut canvas = RgbaImage::from_pixel(w, h, bg);
        let total = p.duration();
        let t = t.clamp(0.0, total.max(0.0));

        let (layers, dip) = self.layers_at(p, t);
        for l in layers {
            self.draw_clip(&mut canvas, p, l);
        }
        if let Some((color, a)) = dip {
            overlay(&mut canvas, color, a);
        }
        for tc in &p.texts {
            if t >= tc.start && t < tc.end() {
                self.draw_text(&mut canvas, tc, t);
            }
        }
        // Whole-show fades.
        if total > 0.0 && !self.skip_edge_fades {
            let fade = EDGE_FADE.min(total * 0.25);
            let mut a = 0.0;
            if p.fade_in_start && t < fade {
                a = 1.0 - smooth(t / fade);
            }
            if p.fade_out_end && t > total - fade {
                a = f64::max(a, smooth((t - (total - fade)) / fade));
            }
            if a > 0.0 {
                overlay(&mut canvas, p.background, a as f32);
            }
        }
        canvas
    }

    /// Which clips are visible at `t`, and any dip-to-colour overlay.
    fn layers_at(&self, p: &Project, t: f64) -> (Vec<Layer>, Option<([u8; 3], f32)>) {
        let n = p.clips.len();
        if n == 0 {
            return (vec![], None);
        }
        let starts = p.clip_starts();
        let i = starts.iter().rposition(|&s| s <= t).unwrap_or(0);
        let one = |c: usize, op: f32| Layer { clip: c, local_t: t - starts[c], opacity: op };

        // Are we inside the transition at the start of clip i, or the end of it?
        let mut boundary = None;
        let d_in = transition_len(p, i);
        if i > 0 && t < starts[i] + d_in / 2.0 {
            boundary = Some((i, d_in));
        } else if i + 1 < n {
            let d_out = transition_len(p, i + 1);
            if t >= starts[i + 1] - d_out / 2.0 {
                boundary = Some((i + 1, d_out));
            }
        }
        let Some((b, d)) = boundary.filter(|(_, d)| *d > 0.0) else {
            return (vec![one(i, 1.0)], None);
        };
        let u = ((t - (starts[b] - d / 2.0)) / d).clamp(0.0, 1.0);
        let (a, c) = (b - 1, b);
        match p.clips[b].transition_in.kind {
            TransitionKind::Cut => (vec![one(if u < 0.5 { a } else { c }, 1.0)], None),
            TransitionKind::Crossfade => (vec![one(a, 1.0), one(c, smooth(u) as f32)], None),
            TransitionKind::FadeInOut => {
                if u < 0.5 {
                    (vec![one(a, 1.0 - smooth(u * 2.0) as f32)], None)
                } else {
                    (vec![one(c, smooth(u * 2.0 - 1.0) as f32)], None)
                }
            }
            TransitionKind::DipToBlack | TransitionKind::DipToWhite => {
                let color = if p.clips[b].transition_in.kind == TransitionKind::DipToBlack {
                    [0, 0, 0]
                } else {
                    [255, 255, 255]
                };
                if u < 0.5 {
                    (vec![one(a, 1.0)], Some((color, smooth(u * 2.0) as f32)))
                } else {
                    (vec![one(c, 1.0)], Some((color, 1.0 - smooth(u * 2.0 - 1.0) as f32)))
                }
            }
        }
    }

    fn image(&mut self, path: &PathBuf, max_side: u32) -> Option<Arc<RgbaImage>> {
        let entry = self
            .images
            .entry((path.clone(), max_side))
            .or_insert_with(|| media::load_image(path, max_side).map(Arc::new).map_err(|e| e.to_string()));
        match entry {
            Ok(img) => Some(img.clone()),
            Err(e) => {
                if !self.errors.contains(e) {
                    self.errors.push(e.clone());
                }
                None
            }
        }
    }

    fn draw_clip(&mut self, canvas: &mut RgbaImage, p: &Project, l: Layer) {
        let (w, h) = canvas.dimensions();
        let clip = &p.clips[l.clip];
        // Leave headroom for zooming in.
        let max_side = ((w.max(h) as f32) * 1.5) as u32;
        let max_side = max_side.next_power_of_two().min(4096).max(256);

        // Motion runs across the clip plus half of each neighbouring transition.
        let lo = -transition_len(p, l.clip) / 2.0;
        let hi = clip.duration + transition_len(p, l.clip + 1) / 2.0;
        let progress = ((l.local_t - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
        let mt = motion_transform(&clip.motion, l.clip, progress, l.local_t, clip.duration);

        let opacity = l.opacity * mt.opacity;
        if opacity <= 0.001 {
            return;
        }
        match clip.kind {
            MediaKind::Image => {
                if let Some(img) = self.image(&clip.path, max_side) {
                    draw_transformed(canvas, &img, clip.fit, mt, opacity);
                }
            }
            MediaKind::Video => {
                let src_t = clip.source_in + l.local_t.clamp(0.0, clip.duration);
                let key = (clip.id, max_side);
                if !self.videos.contains_key(&key) {
                    match VideoDecoder::open(&clip.path, max_side.min(1920), self.video_fps) {
                        Ok(d) => {
                            self.videos.insert(key, d);
                        }
                        Err(e) => {
                            self.note(e);
                            return;
                        }
                    }
                }
                let dec = self.videos.get_mut(&key).unwrap();
                match dec.frame_at(src_t) {
                    Ok(Some(frame)) => draw_transformed(canvas, frame, clip.fit, mt, opacity),
                    Ok(None) => {}
                    Err(e) => self.note(e),
                }
            }
            MediaKind::Audio => {}
        }
    }

    fn note(&mut self, e: MediaError) {
        let s = e.to_string();
        if !self.errors.contains(&s) {
            self.errors.push(s);
        }
    }

    fn draw_text(&mut self, canvas: &mut RgbaImage, tc: &TextClip, t: f64) {
        if tc.text.trim().is_empty() {
            return;
        }
        let (w, h) = canvas.dimensions();
        let px = (tc.size * h as f32).max(4.0);
        let key = TextKey { text: tc.text.clone(), font: tc.font.clone(), px: px.round() as u32, align: tc.align as u8 };
        let mask = self
            .texts
            .entry(key.clone())
            .or_insert_with(|| text::rasterize(&tc.text, &text::fonts().get(&tc.font), px, tc.align))
            .clone();

        // 0 = hidden, 1 = fully shown, for the in- and out-animation.
        let ad = tc.anim_duration.max(0.01).min(tc.duration / 2.0);
        let e_in = smooth((t - tc.start) / ad) as f32;
        let e_out = smooth((tc.end() - t) / ad) as f32;

        let mut alpha = tc.color[3] as f32 / 255.0;
        let mut blur_r = 0.0f32;
        let mut reveal: Vec<(TextAnim, f32)> = vec![];
        for (anim, e) in [(tc.anim_in, e_in), (tc.anim_out, e_out)] {
            if e >= 1.0 {
                continue;
            }
            match anim {
                TextAnim::None => {}
                TextAnim::Fade => alpha *= e,
                TextAnim::Blur => {
                    blur_r = blur_r.max((1.0 - e) * px * 0.3);
                    alpha *= 0.15 + 0.85 * e;
                }
                r => reveal.push((r, e)),
            }
        }
        if alpha <= 0.002 {
            return;
        }
        let blurred;
        let mask_ref: &TextMask = if blur_r >= 1.0 {
            blurred = text::blur(&mask, blur_r);
            &blurred
        } else {
            &mask
        };

        let cx = tc.pos[0] * w as f32;
        let cy = tc.pos[1] * h as f32;
        // Reveal is measured against the un-blurred text box.
        let box_x0 = cx - mask.width as f32 / 2.0;
        let box_y0 = cy - mask.height as f32 / 2.0;
        let rv = Reveal { x0: box_x0, y0: box_y0, w: mask.width as f32, h: mask.height as f32, steps: reveal };

        if tc.shadow {
            let sh = self
                .shadows
                .entry(key)
                .or_insert_with(|| Arc::new(text::blur(&mask, (px * 0.05).max(1.0))))
                .clone();
            let off = px * 0.04;
            let sh_ref: &TextMask = if blur_r >= 1.0 { mask_ref } else { &sh };
            draw_mask(
                canvas,
                sh_ref,
                cx - sh_ref.width as f32 / 2.0 + off,
                cy - sh_ref.height as f32 / 2.0 + off,
                [0, 0, 0],
                alpha * 0.55,
                &rv,
            );
        }
        draw_mask(
            canvas,
            mask_ref,
            cx - mask_ref.width as f32 / 2.0,
            cy - mask_ref.height as f32 / 2.0,
            [tc.color[0], tc.color[1], tc.color[2]],
            alpha,
            &rv,
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MotionTransform {
    pub scale: f32,
    /// -1..1 of the available slack on each axis.
    pub pan: [f32; 2],
    pub opacity: f32,
}

/// Where the picture sits at `progress` (0..1 through its time on screen).
pub fn motion_transform(m: &Motion, index: usize, progress: f64, local_t: f64, dur: f64) -> MotionTransform {
    let speed = m.speed.clamp(0.1, 5.0) as f64;
    let p = (progress * speed).min(1.0) as f32;
    let i = m.intensity.clamp(0.0, 1.0);
    let mut mt = MotionTransform { scale: 1.0, pan: [0.0, 0.0], opacity: 1.0 };
    match m.kind {
        MotionKind::None => {}
        MotionKind::ZoomIn => mt.scale = 1.0 + 0.35 * i * smooth(p as f64) as f32,
        MotionKind::ZoomOut => mt.scale = 1.0 + 0.35 * i * (1.0 - smooth(p as f64) as f32),
        MotionKind::KenBurns => {
            let dir = match m.direction {
                PanDirection::Auto => [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]][index % 4],
                PanDirection::Left => [-1.0, 0.0],
                PanDirection::Right => [1.0, 0.0],
                PanDirection::Up => [0.0, -1.0],
                PanDirection::Down => [0.0, 1.0],
            };
            // Alternate zooming in and out so a run of pictures feels varied.
            let z = if index % 2 == 0 { p } else { 1.0 - p };
            mt.scale = 1.08 + 0.25 * i * z;
            // Moving "right" means the camera pans right, so the picture slides left.
            let travel = (2.0 * p - 1.0) * (0.35 + 0.6 * i);
            mt.pan = [-dir[0] * travel, -dir[1] * travel];
        }
        MotionKind::Fade => {
            let fd = (dur * 0.25 / speed).min(dur * 0.5).max(0.01);
            let ramp = (local_t / fd).min((dur - local_t) / fd).clamp(0.0, 1.0);
            mt.opacity = 1.0 - i * (1.0 - smooth(ramp) as f32);
        }
    }
    mt
}

/// Draw `src` scaled/panned into `canvas` with bilinear sampling.
fn draw_transformed(canvas: &mut RgbaImage, src: &RgbaImage, fit: Fit, mt: MotionTransform, opacity: f32) {
    let (w, h) = (canvas.width() as f32, canvas.height() as f32);
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    if iw < 1.0 || ih < 1.0 {
        return;
    }
    let k = match fit {
        Fit::Fill => (w / iw).max(h / ih),
        Fit::Fit => (w / iw).min(h / ih),
    } * mt.scale;
    let slack_x = ((iw * k - w) / 2.0).max(0.0);
    let slack_y = ((ih * k - h) / 2.0).max(0.0);
    let off_x = mt.pan[0].clamp(-1.0, 1.0) * slack_x;
    let off_y = mt.pan[1].clamp(-1.0, 1.0) * slack_y;
    let inv = 1.0 / k;
    let sw = src.width() as i32;
    let sh = src.height() as i32;
    let sraw = src.as_raw();
    let row_len = canvas.width() as usize * 4;

    canvas.par_chunks_mut(row_len).enumerate().for_each(|(y, row)| {
        let sy = (y as f32 + 0.5 - h / 2.0 - off_y) * inv + ih / 2.0 - 0.5;
        if sy < -1.0 || sy > ih {
            return;
        }
        let y0 = sy.floor() as i32;
        let fy = sy - y0 as f32;
        for x in 0..row.len() / 4 {
            let sx = (x as f32 + 0.5 - w / 2.0 - off_x) * inv + iw / 2.0 - 0.5;
            if sx < -1.0 || sx > iw {
                continue;
            }
            let x0 = sx.floor() as i32;
            let fx = sx - x0 as f32;
            let mut acc = [0f32; 4];
            for (dx, dy, wgt) in
                [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
            {
                let px = (x0 + dx).clamp(0, sw - 1);
                let py = (y0 + dy).clamp(0, sh - 1);
                let edge = (x0 + dx) >= 0 && (x0 + dx) < sw && (y0 + dy) >= 0 && (y0 + dy) < sh;
                let i = ((py * sw + px) * 4) as usize;
                let a = if edge { sraw[i + 3] as f32 / 255.0 } else { 0.0 } * wgt;
                acc[0] += sraw[i] as f32 * a;
                acc[1] += sraw[i + 1] as f32 * a;
                acc[2] += sraw[i + 2] as f32 * a;
                acc[3] += a;
            }
            let a = acc[3] * opacity;
            if a <= 0.0 {
                continue;
            }
            let d = &mut row[x * 4..x * 4 + 4];
            for c in 0..3 {
                let s = acc[c] / acc[3];
                d[c] = (d[c] as f32 * (1.0 - a) + s * a).round() as u8;
            }
        }
    });
}

/// Blend the whole canvas toward `color`.
fn overlay(canvas: &mut RgbaImage, color: [u8; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    canvas.par_chunks_mut(4).for_each(|px| {
        for c in 0..3 {
            px[c] = (px[c] as f32 * (1.0 - a) + color[c] as f32 * a).round() as u8;
        }
    });
}

struct Reveal {
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
    steps: Vec<(TextAnim, f32)>,
}

impl Reveal {
    /// Visibility (0..1) of canvas point (x, y) under the reveal masks, with a soft edge.
    fn at(&self, x: f32, y: f32) -> f32 {
        let mut v = 1.0f32;
        for &(anim, e) in &self.steps {
            let fx = (x - self.x0) / self.w;
            let fy = (y - self.y0) / self.h;
            let soft = 0.08;
            // Edge travels from just outside one side to just past the other.
            let edge = e * (1.0 + soft) - soft / 2.0;
            let d = match anim {
                TextAnim::RevealLeft => edge - fx,
                TextAnim::RevealRight => fx - (1.0 - edge),
                TextAnim::RevealTop => edge - fy,
                TextAnim::RevealBottom => fy - (1.0 - edge),
                _ => 1.0,
            };
            v *= (d / soft + 0.5).clamp(0.0, 1.0);
        }
        v
    }
}

fn draw_mask(canvas: &mut RgbaImage, mask: &TextMask, x0: f32, y0: f32, color: [u8; 3], alpha: f32, rv: &Reveal) {
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);
    let ox = x0.round() as i32;
    let oy = y0.round() as i32;
    for my in 0..mask.height as i32 {
        let y = oy + my;
        if y < 0 || y >= ch {
            continue;
        }
        for mx in 0..mask.width as i32 {
            let x = ox + mx;
            if x < 0 || x >= cw {
                continue;
            }
            let cov = mask.coverage[(my as u32 * mask.width + mx as u32) as usize];
            if cov <= 0.0 {
                continue;
            }
            let mut a = cov * alpha;
            if !rv.steps.is_empty() {
                a *= rv.at(x as f32 + 0.5, y as f32 + 0.5);
            }
            if a <= 0.0 {
                continue;
            }
            let p = canvas.get_pixel_mut(x as u32, y as u32);
            for c in 0..3 {
                p.0[c] = (p.0[c] as f32 * (1.0 - a) + color[c] as f32 * a).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(path: &std::path::Path, color: [u8; 3]) {
        RgbaImage::from_pixel(64, 36, Rgba([color[0], color[1], color[2], 255])).save(path).unwrap();
    }

    fn two_clip_project(kind: TransitionKind) -> (Project, tempdir::Dir) {
        let dir = tempdir::Dir::new();
        let a = dir.path.join("red.png");
        let b = dir.path.join("blue.png");
        solid(&a, [255, 0, 0]);
        solid(&b, [0, 0, 255]);
        let mut p = Project { fade_in_start: false, fade_out_end: false, ..Default::default() };
        let mut ca = Clip::new(a, MediaKind::Image, 4.0);
        let mut cb = Clip::new(b, MediaKind::Image, 4.0);
        ca.motion.kind = MotionKind::None;
        cb.motion.kind = MotionKind::None;
        cb.transition_in = Transition { kind, duration: 1.0 };
        p.clips = vec![ca, cb];
        (p, dir)
    }

    mod tempdir {
        pub struct Dir {
            pub path: std::path::PathBuf,
        }
        impl Dir {
            pub fn new() -> Self {
                let path = std::env::temp_dir()
                    .join(format!("ssm-test-{}-{}", std::process::id(), crate::model::new_id()));
                std::fs::create_dir_all(&path).unwrap();
                Self { path }
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    fn center(img: &RgbaImage) -> [u8; 4] {
        img.get_pixel(img.width() / 2, img.height() / 2).0
    }

    #[test]
    fn crossfade_blends_midway() {
        let (p, _d) = two_clip_project(TransitionKind::Crossfade);
        let mut r = Renderer::new();
        assert_eq!(center(&r.render(&p, 1.0, 32, 18))[..3], [255, 0, 0]);
        assert_eq!(center(&r.render(&p, 6.0, 32, 18))[..3], [0, 0, 255]);
        let mid = center(&r.render(&p, 4.0, 32, 18));
        assert!(mid[0] > 100 && mid[2] > 100, "{mid:?}");
    }

    #[test]
    fn dip_to_white_hits_white() {
        let (p, _d) = two_clip_project(TransitionKind::DipToWhite);
        let mut r = Renderer::new();
        assert_eq!(center(&r.render(&p, 4.0, 32, 18))[..3], [255, 255, 255]);
    }

    #[test]
    fn dip_to_black_hits_black() {
        let (p, _d) = two_clip_project(TransitionKind::DipToBlack);
        let mut r = Renderer::new();
        assert_eq!(center(&r.render(&p, 4.0, 32, 18))[..3], [0, 0, 0]);
    }

    #[test]
    fn fade_in_out_hits_background() {
        let (mut p, _d) = two_clip_project(TransitionKind::FadeInOut);
        p.background = [10, 200, 10];
        let mut r = Renderer::new();
        assert_eq!(center(&r.render(&p, 4.0, 32, 18))[..3], [10, 200, 10]);
    }

    #[test]
    fn text_draws_pixels() {
        let mut p = Project::default();
        let mut t = TextClip::new(0.0);
        t.text = "Hello".into();
        t.anim_in = TextAnim::None;
        t.shadow = false;
        p.texts.push(t);
        let mut r = Renderer::new();
        let img = r.render(&p, 2.0, 320, 180);
        assert!(img.pixels().any(|px| px.0[0] > 200));
    }

    #[test]
    fn reveal_from_left_shows_left_half_first() {
        let mut p = Project { fade_in_start: false, ..Default::default() };
        let mut t = TextClip::new(0.0);
        t.text = "MMMMMMMM".into();
        t.size = 0.3;
        t.shadow = false;
        t.anim_in = TextAnim::RevealLeft;
        t.anim_duration = 2.0;
        p.texts.push(t);
        let mut r = Renderer::new();
        let img = r.render(&p, 1.0, 320, 180);
        let lit = |x0: u32, x1: u32| (x0..x1).flat_map(|x| (0..180).map(move |y| (x, y))).filter(|&(x, y)| img.get_pixel(x, y).0[0] > 128).count();
        assert!(lit(0, 160) > 10 * lit(160, 320).max(1), "{} vs {}", lit(0, 160), lit(160, 320));
    }
}
