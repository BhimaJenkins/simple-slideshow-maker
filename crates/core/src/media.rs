//! Loading pictures, probing media and decoding video frames.
//!
//! Still images are decoded in pure Rust (works on every platform). Video,
//! audio probing, uncommon image formats (HEIC/AVIF) and MP4 export use the
//! `ffmpeg`/`ffprobe` command line tools when they are available.

use crate::model::MediaKind;
use image::RgbaImage;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("unsupported file type: {0}")]
    Unsupported(String),
    #[error("could not read {path}: {msg}")]
    Decode { path: String, msg: String },
    #[error("this needs FFmpeg, which was not found. Install FFmpeg or put it next to the app.")]
    NoFfmpeg,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub const IMAGE_EXTS: &[&str] =
    &["png", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "tif", "tiff", "tga", "ico", "pnm", "ppm", "heic", "heif", "avif"];
pub const VIDEO_EXTS: &[&str] =
    &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg", "3gp", "flv", "mts", "m2ts", "ogv"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "wma", "aif", "aiff", "alac"];

/// Guess what a file is from its extension.
pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else if AUDIO_EXTS.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else {
        None
    }
}

fn find_tool(name: &str, env: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(env) {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    // Next to the app (how installers bundle it).
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            for cand in [dir.join(&exe), dir.join("ffmpeg").join(&exe), dir.join("../Resources").join(&exe)] {
                if cand.exists() {
                    return Some(cand);
                }
            }
        }
    }
    // On PATH.
    let ok = Command::new(&exe)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then(|| PathBuf::from(exe))
}

/// Path to `ffmpeg`, if present. Override with `SSM_FFMPEG`.
pub fn ffmpeg() -> Option<&'static Path> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| find_tool("ffmpeg", "SSM_FFMPEG")).as_deref()
}

/// Path to `ffprobe`, if present. Override with `SSM_FFPROBE`.
pub fn ffprobe() -> Option<&'static Path> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| find_tool("ffprobe", "SSM_FFPROBE")).as_deref()
}

pub(crate) fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Length of an audio or video file in seconds.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let probe = ffprobe()?;
    let out = quiet(Command::new(probe).args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]))
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0)
}

/// Pixel size of the first video stream.
pub fn probe_video_size(path: &Path) -> Option<(u32, u32)> {
    let probe = ffprobe()?;
    let out = quiet(Command::new(probe).args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height",
        "-of",
        "csv=s=x:p=0",
    ]))
    .arg(path)
    .stdin(Stdio::null())
    .output()
    .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s.trim().lines().next()?.split('x');
    Some((it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?))
}

/// Decode a still image, downscaling so neither side exceeds `max_side`.
pub fn load_image(path: &Path, max_side: u32) -> Result<RgbaImage, MediaError> {
    let decoded = (|| -> Result<image::DynamicImage, image::ImageError> {
        use image::ImageDecoder;
        let mut dec = image::ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
        let orientation = dec.orientation().ok();
        let mut img = image::DynamicImage::from_decoder(dec)?;
        if let Some(o) = orientation {
            img.apply_orientation(o);
        }
        Ok(img)
    })();
    match decoded {
        Ok(img) => Ok(downscale(img.to_rgba8(), max_side)),
        // HEIC/AVIF and friends: let FFmpeg convert them.
        Err(e) => ffmpeg_still(path, 0.0, max_side)
            .map_err(|_| MediaError::Decode { path: path.display().to_string(), msg: e.to_string() }),
    }
}

pub fn downscale(img: RgbaImage, max_side: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w <= max_side && h <= max_side {
        return img;
    }
    let k = max_side as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
}

/// One frame of a video (or any FFmpeg-readable picture) at time `t`.
pub fn ffmpeg_still(path: &Path, t: f64, max_side: u32) -> Result<RgbaImage, MediaError> {
    let ff = ffmpeg().ok_or(MediaError::NoFfmpeg)?;
    let scale = format!("scale='min({max_side},iw)':'min({max_side},ih)':force_original_aspect_ratio=decrease");
    let out = quiet(Command::new(ff).args(["-v", "error", "-ss", &format!("{t:.3}"), "-i"]))
        .arg(path)
        .args(["-frames:v", "1", "-vf", &scale, "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .output()?;
    let img = image::load_from_memory(&out.stdout)
        .map_err(|e| MediaError::Decode { path: path.display().to_string(), msg: e.to_string() })?;
    Ok(img.to_rgba8())
}

/// Streams frames out of a video file at a fixed rate. Seeking forward a little
/// reads ahead; anything else restarts FFmpeg at the new position.
pub struct VideoDecoder {
    path: PathBuf,
    w: u32,
    h: u32,
    fps: f64,
    child: Option<(Child, ChildStdout)>,
    /// Source time of `frame`.
    pos: f64,
    frame: Option<RgbaImage>,
    eof: bool,
}

impl VideoDecoder {
    pub fn open(path: &Path, max_side: u32, fps: f64) -> Result<Self, MediaError> {
        ffmpeg().ok_or(MediaError::NoFfmpeg)?;
        let (sw, sh) = probe_video_size(path).unwrap_or((1280, 720));
        let k = (max_side as f64 / sw.max(sh) as f64).min(1.0);
        // Even sizes keep every pixel format happy.
        let w = (((sw as f64 * k) as u32) & !1).max(2);
        let h = (((sh as f64 * k) as u32) & !1).max(2);
        Ok(Self { path: path.to_owned(), w, h, fps, child: None, pos: f64::NEG_INFINITY, frame: None, eof: false })
    }

    fn start(&mut self, t: f64) -> Result<(), MediaError> {
        self.stop();
        let ff = ffmpeg().ok_or(MediaError::NoFfmpeg)?;
        let vf = format!("fps={},scale={}:{}", self.fps, self.w, self.h);
        let mut child = quiet(Command::new(ff).args(["-v", "error", "-ss", &format!("{:.3}", t.max(0.0)), "-i"]))
            .arg(&self.path)
            .args(["-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let out = child.stdout.take().expect("piped stdout");
        self.child = Some((child, out));
        self.pos = t.max(0.0) - 1.0 / self.fps;
        self.eof = false;
        Ok(())
    }

    fn read_one(&mut self) -> bool {
        let Some((_, out)) = self.child.as_mut() else { return false };
        let mut buf = vec![0u8; (self.w * self.h * 4) as usize];
        if out.read_exact(&mut buf).is_err() {
            self.eof = true;
            return false;
        }
        self.frame = RgbaImage::from_raw(self.w, self.h, buf);
        self.pos += 1.0 / self.fps;
        true
    }

    /// Frame at source time `t` (seconds). Holds the last frame past the end.
    pub fn frame_at(&mut self, t: f64) -> Result<Option<&RgbaImage>, MediaError> {
        let step = 1.0 / self.fps;
        let behind = t < self.pos - step * 0.5;
        let far_ahead = t > self.pos + 3.0;
        if self.child.is_none() || behind || (far_ahead && !self.eof) {
            if !(self.eof && t >= self.pos) {
                self.start(t)?;
            }
        }
        while !self.eof && self.pos + step * 0.5 < t {
            if !self.read_one() {
                break;
            }
        }
        if self.frame.is_none() && !self.eof {
            self.read_one();
        }
        Ok(self.frame.as_ref())
    }

    fn stop(&mut self) {
        if let Some((mut c, _)) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Small preview picture for the timeline / media bin.
pub fn thumbnail(path: &Path, kind: MediaKind, side: u32) -> Option<RgbaImage> {
    match kind {
        MediaKind::Image => load_image(path, side).ok(),
        MediaKind::Video => ffmpeg_still(path, 0.5, side).or_else(|_| ffmpeg_still(path, 0.0, side)).ok(),
        MediaKind::Audio => None,
    }
}
