//! Fonts and text rasterisation.

use crate::model::TextAlign;
use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const DEFAULT_FONT: &str = "Default";
pub const MONO_FONT: &str = "Monospace";

/// Built-in fonts plus every font installed on the system.
pub struct FontLibrary {
    db: fontdb::Database,
    families: Vec<String>,
    cache: Mutex<HashMap<String, FontArc>>,
}

impl FontLibrary {
    fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut families: Vec<String> = db
            .faces()
            .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
            .filter(|n| !n.starts_with('.'))
            .collect();
        families.sort_by_key(|a| a.to_lowercase());
        families.dedup();
        let mut all = vec![DEFAULT_FONT.to_string(), MONO_FONT.to_string()];
        all.extend(families);
        Self { db, families: all, cache: Mutex::new(HashMap::new()) }
    }

    /// Font family names to offer in the UI.
    pub fn families(&self) -> &[String] {
        &self.families
    }

    /// Load a font by family name; falls back to the built-in font.
    pub fn get(&self, family: &str) -> FontArc {
        let mut cache = self.cache.lock().unwrap();
        if let Some(f) = cache.get(family) {
            return f.clone();
        }
        let font = self.load(family).unwrap_or_else(builtin_default);
        cache.insert(family.to_string(), font.clone());
        font
    }

    fn load(&self, family: &str) -> Option<FontArc> {
        match family {
            DEFAULT_FONT => return Some(builtin_default()),
            MONO_FONT => return FontArc::try_from_slice(epaint_default_fonts::HACK_REGULAR).ok(),
            _ => {}
        }
        let id = self.db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        })?;
        self.db
            .with_face_data(id, |data, index| FontVec::try_from_vec_and_index(data.to_vec(), index).ok())
            .flatten()
            .map(FontArc::new)
    }
}

fn builtin_default() -> FontArc {
    FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font")
}

/// The shared font library (scans system fonts on first use).
pub fn fonts() -> &'static FontLibrary {
    static LIB: OnceLock<FontLibrary> = OnceLock::new();
    LIB.get_or_init(FontLibrary::new)
}

/// A rasterised block of text: one coverage value (0..1) per pixel.
pub struct TextMask {
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<f32>,
}

/// Rasterise `text` at `px` pixels tall. Lines are split on `\n`.
pub fn rasterize(text: &str, font: &FontArc, px: f32, align: TextAlign) -> Arc<TextMask> {
    let scale = PxScale::from(px.max(1.0));
    let sf = font.as_scaled(scale);
    let line_h = sf.ascent() - sf.descent() + sf.line_gap();
    let lines: Vec<&str> = text.split('\n').collect();

    let line_width = |line: &str| {
        let mut w = 0.0;
        let mut prev = None;
        for ch in line.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let widths: Vec<f32> = lines.iter().map(|l| line_width(l)).collect();
    let pad = (px * 0.15).ceil();
    let max_w = widths.iter().cloned().fold(0.0, f32::max);
    let width = (max_w + 2.0 * pad).ceil().max(1.0) as u32;
    let height = (line_h * lines.len() as f32 + 2.0 * pad).ceil().max(1.0) as u32;
    let mut coverage = vec![0f32; (width * height) as usize];

    for (li, line) in lines.iter().enumerate() {
        let x0 = pad
            + match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (max_w - widths[li]) / 2.0,
                TextAlign::Right => max_w - widths[li],
            };
        let baseline = pad + sf.ascent() + line_h * li as f32;
        let mut x = x0;
        let mut prev = None;
        for ch in line.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                og.draw(|gx, gy, c| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as u32) < width && (py as u32) < height {
                        let i = (py as u32 * width + px as u32) as usize;
                        coverage[i] = (coverage[i] + c).min(1.0);
                    }
                });
            }
        }
    }
    Arc::new(TextMask { width, height, coverage })
}

/// Separable box blur (run three times ≈ gaussian).
pub fn blur(mask: &TextMask, radius: f32) -> TextMask {
    let r = radius.round() as i32;
    if r < 1 {
        return TextMask { width: mask.width, height: mask.height, coverage: mask.coverage.clone() };
    }
    // Grow the canvas so the blur can spread past the glyph box.
    let grow = (r * 3) as u32;
    let (w, h) = (mask.width + grow * 2, mask.height + grow * 2);
    let mut a = vec![0f32; (w * h) as usize];
    for y in 0..mask.height {
        for x in 0..mask.width {
            a[((y + grow) * w + x + grow) as usize] = mask.coverage[(y * mask.width + x) as usize];
        }
    }
    let mut b = vec![0f32; a.len()];
    for _ in 0..3 {
        box_pass(&a, &mut b, w as i32, h as i32, r, true);
        box_pass(&b, &mut a, w as i32, h as i32, r, false);
    }
    TextMask { width: w, height: h, coverage: a }
}

fn box_pass(src: &[f32], dst: &mut [f32], w: i32, h: i32, r: i32, horizontal: bool) {
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let norm = 1.0 / (2 * r + 1) as f32;
    for o in 0..outer {
        let idx = |i: i32| -> usize {
            let i = i.clamp(0, inner - 1);
            if horizontal { (o * w + i) as usize } else { (i * w + o) as usize }
        };
        let mut acc = 0.0;
        for i in -r..=r {
            acc += src[idx(i)];
        }
        for i in 0..inner {
            dst[idx(i)] = acc * norm;
            acc += src[idx(i + r + 1)] - src[idx(i - r)];
        }
    }
}
