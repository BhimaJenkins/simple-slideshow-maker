//! MP4 (H.264 + AAC, via FFmpeg) and animated GIF (pure Rust) export.

use crate::media::{self, quiet};
use crate::model::{PlaybackMode, Project};
use crate::render::{Renderer, EDGE_FADE};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExportFormat {
    #[default]
    Mp4,
    Gif,
}

impl ExportFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Mp4 => "mp4",
            ExportFormat::Gif => "gif",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Mp4 => "MP4 video (with music)",
            ExportFormat::Gif => "Animated GIF (no sound)",
        }
    }
}

/// Simple quality / file size choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Quality {
    /// Smallest file.
    Small,
    #[default]
    Balanced,
    /// Best looking.
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Small, Quality::Balanced, Quality::High];
    pub fn label(self) -> &'static str {
        match self {
            Quality::Small => "Small file",
            Quality::Balanced => "Balanced",
            Quality::High => "High quality",
        }
    }
    fn crf(self) -> u32 {
        match self {
            Quality::Small => 30,
            Quality::Balanced => 24,
            Quality::High => 19,
        }
    }
    fn audio_kbps(self) -> u32 {
        match self {
            Quality::Small => 96,
            Quality::Balanced => 128,
            Quality::High => 192,
        }
    }
    /// Rough bits per pixel per frame that x264 lands on at this CRF for photo slideshows.
    fn bpp(self) -> f64 {
        match self {
            Quality::Small => 0.02,
            Quality::Balanced => 0.045,
            Quality::High => 0.1,
        }
    }
    /// GIF palette search effort: lower is better looking but slower.
    fn gif_speed(self) -> i32 {
        match self {
            Quality::Small => 20,
            Quality::Balanced => 10,
            Quality::High => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub format: ExportFormat,
    /// Output height in pixels; width follows the project's aspect ratio.
    pub height: u32,
    pub fps: u32,
    pub quality: Quality,
    /// MP4 only: aim for roughly this many megabytes instead of a quality level.
    pub target_mb: Option<f64>,
}

impl ExportSettings {
    pub fn for_format(format: ExportFormat) -> Self {
        match format {
            ExportFormat::Mp4 => Self { format, height: 1080, fps: 30, quality: Quality::Balanced, target_mb: None },
            ExportFormat::Gif => Self { format, height: 360, fps: 12, quality: Quality::Balanced, target_mb: None },
        }
    }

    pub fn size(&self, p: &Project) -> (u32, u32) {
        let h = self.height.max(16) & !1;
        let w = ((h as f32 * p.aspect()).round() as u32).max(16) & !1;
        (w, h)
    }

    /// Rough output size in bytes, for showing before export.
    pub fn estimate_bytes(&self, p: &Project) -> u64 {
        let dur = p.duration();
        let (w, h) = self.size(p);
        let frames = dur * self.fps as f64;
        match self.format {
            ExportFormat::Mp4 => {
                if let Some(mb) = self.target_mb {
                    return (mb * 1024.0 * 1024.0) as u64;
                }
                let video = w as f64 * h as f64 * frames * self.quality.bpp() / 8.0;
                let audio = if p.music.is_empty() { 0.0 } else { self.quality.audio_kbps() as f64 * 1000.0 / 8.0 * dur };
                (video + audio) as u64
            }
            // LZW on photographic frames lands around 0.4-0.6 bytes per pixel.
            ExportFormat::Gif => (w as f64 * h as f64 * frames * 0.5) as u64,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("nothing to export: add some pictures first")]
    Empty,
    #[error("MP4 export needs FFmpeg, which was not found. Install FFmpeg, or export a GIF instead.")]
    NoFfmpeg,
    #[error("export cancelled")]
    Cancelled,
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("GIF encoding failed: {0}")]
    Gif(String),
}

/// Export `p` to `out`. `progress` gets 0..1 and returns `false` to cancel.
pub fn export(
    p: &Project,
    settings: &ExportSettings,
    out: &Path,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<PathBuf, ExportError> {
    if p.duration() <= 0.0 {
        return Err(ExportError::Empty);
    }
    let result = match settings.format {
        ExportFormat::Mp4 => export_mp4(p, settings, out, progress),
        ExportFormat::Gif => export_gif(p, settings, out, progress),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result.map(|_| out.to_path_buf())
}

fn frame_count(p: &Project, fps: u32) -> usize {
    ((p.duration() * fps as f64).round() as usize).max(1)
}

fn has_encoder(ff: &Path, name: &str) -> bool {
    quiet(Command::new(ff).args(["-hide_banner", "-encoders"]))
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false)
}

/// The FFmpeg audio filter that lays the music clips out on the timeline.
pub fn audio_filter(p: &Project, first_input: usize) -> Option<String> {
    let total = p.duration();
    let clips: Vec<_> = p.music.iter().filter(|m| m.start < total && m.duration > 0.0).collect();
    if clips.is_empty() {
        return None;
    }
    let mut f = String::new();
    for (k, m) in clips.iter().enumerate() {
        let dur = m.duration.min(total - m.start);
        let fi = m.fade_in.clamp(0.0, dur / 2.0);
        let fo = m.fade_out.clamp(0.0, dur / 2.0);
        let delay_ms = (m.start.max(0.0) * 1000.0).round() as u64;
        f += &format!(
            "[{inp}:a]atrim=start={si:.3}:duration={dur:.3},asetpts=PTS-STARTPTS,aformat=sample_rates=48000:channel_layouts=stereo,\
             afade=t=in:st=0:d={fi:.3},afade=t=out:st={fos:.3}:d={fo:.3},volume={vol:.3},adelay={delay_ms}:all=1[m{k}];",
            inp = first_input + k,
            si = m.source_in.max(0.0),
            fos = (dur - fo).max(0.0),
            vol = m.volume.max(0.0),
        );
    }
    let ins: String = (0..clips.len()).map(|k| format!("[m{k}]")).collect();
    let end_fade = if p.fade_out_end {
        let d = EDGE_FADE.min(total * 0.25);
        format!(",afade=t=out:st={:.3}:d={:.3}", total - d, d)
    } else {
        String::new()
    };
    f += &format!("{ins}amix=inputs={}:normalize=0:duration=longest,atrim=0:{total:.3}{end_fade}[aout]", clips.len());
    Some(f)
}

fn export_mp4(
    p: &Project,
    s: &ExportSettings,
    out: &Path,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<(), ExportError> {
    let ff = media::ffmpeg().ok_or(ExportError::NoFfmpeg)?;
    let (w, h) = s.size(p);
    let fps = s.fps.max(1);
    let total = p.duration();

    let mut cmd = Command::new(ff);
    quiet(&mut cmd).args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}")]);
    cmd.args(["-r", &fps.to_string(), "-i", "-"]);
    let music: Vec<_> = p.music.iter().filter(|m| m.start < total && m.duration > 0.0).collect();
    for m in &music {
        cmd.arg("-i").arg(&m.path);
    }
    if let Some(filter) = audio_filter(p, 1) {
        cmd.args(["-filter_complex", &filter, "-map", "0:v", "-map", "[aout]"]);
        cmd.args(["-c:a", "aac", "-b:a", &format!("{}k", s.quality.audio_kbps())]);
    }
    if has_encoder(ff, "libx264") {
        cmd.args(["-c:v", "libx264", "-preset", "medium"]);
        match s.target_mb {
            Some(mb) => {
                let audio = if music.is_empty() { 0.0 } else { s.quality.audio_kbps() as f64 * 1000.0 };
                let kbps = (((mb * 8.0 * 1024.0 * 1024.0) / total - audio) / 1000.0 * 0.95).max(100.0) as u32;
                cmd.args(["-b:v", &format!("{kbps}k"), "-maxrate", &format!("{}k", kbps * 3 / 2)]);
                cmd.args(["-bufsize", &format!("{}k", kbps * 2)]);
            }
            None => {
                cmd.args(["-crf", &s.quality.crf().to_string()]);
            }
        }
    } else {
        cmd.args(["-c:v", "mpeg4", "-q:v", "4"]);
    }
    cmd.args(["-pix_fmt", "yuv420p", "-movflags", "+faststart", "-t", &format!("{total:.3}")]);
    cmd.arg(out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let mut r = Renderer::new();
    r.video_fps = fps as f64;
    let n = frame_count(p, fps);
    let mut write_err = None;
    for i in 0..n {
        if !progress(i as f32 / n as f32) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ExportError::Cancelled);
        }
        let frame = r.render(p, i as f64 / fps as f64, w, h);
        if let Err(e) = stdin.write_all(frame.as_raw()) {
            write_err = Some(e);
            break;
        }
    }
    drop(stdin);
    let status = child.wait()?;
    let log = err_thread.join().unwrap_or_default();
    if !status.success() || write_err.is_some() {
        let msg = log.lines().last().unwrap_or("unknown error").to_string();
        return Err(ExportError::Ffmpeg(msg));
    }
    progress(1.0);
    Ok(())
}

fn export_gif(
    p: &Project,
    s: &ExportSettings,
    out: &Path,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<(), ExportError> {
    let (w, h) = s.size(p);
    let fps = s.fps.clamp(1, 50);
    let file = BufWriter::new(File::create(out)?);
    let mut enc = gif::Encoder::new(file, w as u16, h as u16, &[]).map_err(|e| ExportError::Gif(e.to_string()))?;
    let repeat = match p.playback {
        PlaybackMode::Loop => gif::Repeat::Infinite,
        PlaybackMode::Once => gif::Repeat::Finite(0),
    };
    enc.set_repeat(repeat).map_err(|e| ExportError::Gif(e.to_string()))?;

    let mut r = Renderer::new();
    r.video_fps = fps as f64;
    let n = frame_count(p, fps);
    let speed = s.quality.gif_speed();
    // GIF delays are in 1/100 s; carry the rounding so long shows stay in sync.
    let mut clock_cs = 0u64;
    let batch = rayon::current_num_threads().max(2) * 2;
    let mut i = 0;
    while i < n {
        if !progress(i as f32 / n as f32) {
            return Err(ExportError::Cancelled);
        }
        let end = (i + batch).min(n);
        let frames: Vec<_> = (i..end).map(|k| r.render(p, k as f64 / fps as f64, w, h)).collect();
        let encoded: Vec<gif::Frame<'static>> = frames
            .into_par_iter()
            .map(|img| {
                let mut raw = img.into_raw();
                gif::Frame::from_rgba_speed(w as u16, h as u16, &mut raw, speed)
            })
            .collect();
        for (j, mut frame) in encoded.into_iter().enumerate() {
            let k = (i + j) as u64 + 1;
            let target = k * 100 / fps as u64;
            frame.delay = (target - clock_cs) as u16;
            clock_cs = target;
            enc.write_frame(&frame).map_err(|e| ExportError::Gif(e.to_string()))?;
        }
        i = end;
    }
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn audio_filter_places_clips() {
        let mut p = Project::default();
        p.clips.push(Clip::new("a.png".into(), MediaKind::Image, 10.0));
        let mut m = AudioClip::new("m.mp3".into(), 2.0, Some(60.0));
        m.duration = 8.0;
        p.music.push(m);
        let f = audio_filter(&p, 1).unwrap();
        assert!(f.contains("[1:a]atrim=start=0.000:duration=8.000"));
        assert!(f.contains("adelay=2000:all=1"));
        assert!(f.contains("amix=inputs=1"));
    }

    #[test]
    fn sizes_are_even_and_follow_aspect() {
        let p = Project::default();
        let s = ExportSettings { height: 721, ..ExportSettings::for_format(ExportFormat::Mp4) };
        assert_eq!(s.size(&p), (1280, 720));
    }
}
