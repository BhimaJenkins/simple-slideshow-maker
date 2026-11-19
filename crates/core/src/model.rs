//! The project model: everything that is saved in a `.ssm` project file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A process-unique id for timeline items (used for selection and caching).
pub fn new_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Make sure ids handed out later never collide with ids loaded from disk.
pub fn bump_ids_past(max: u64) {
    NEXT_ID.fetch_max(max + 1, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaybackMode {
    #[default]
    Loop,
    Once,
}

/// How a picture is fitted into the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fit {
    /// Fill the whole frame, cropping the edges if needed.
    #[default]
    Fill,
    /// Show the whole picture, with bars if needed.
    Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransitionKind {
    /// Hard cut.
    Cut,
    /// Fade the outgoing picture out to the background colour, then fade the next one in.
    FadeInOut,
    /// Blend directly from one picture to the next.
    #[default]
    Crossfade,
    DipToBlack,
    DipToWhite,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 5] = [
        TransitionKind::Cut,
        TransitionKind::FadeInOut,
        TransitionKind::Crossfade,
        TransitionKind::DipToBlack,
        TransitionKind::DipToWhite,
    ];
    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Cut => "Cut",
            TransitionKind::FadeInOut => "Fade in/out",
            TransitionKind::Crossfade => "Cross fade",
            TransitionKind::DipToBlack => "Dip to black",
            TransitionKind::DipToWhite => "Dip to white",
        }
    }
}

/// The transition *into* a clip from the clip before it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Seconds. Centred on the cut point.
    pub duration: f64,
}

impl Default for Transition {
    fn default() -> Self {
        Self { kind: TransitionKind::Crossfade, duration: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MotionKind {
    None,
    ZoomIn,
    ZoomOut,
    #[default]
    KenBurns,
    /// Fade the picture in and out within its own time on screen.
    Fade,
}

impl MotionKind {
    pub const ALL: [MotionKind; 5] =
        [MotionKind::None, MotionKind::ZoomIn, MotionKind::ZoomOut, MotionKind::KenBurns, MotionKind::Fade];
    pub fn label(self) -> &'static str {
        match self {
            MotionKind::None => "None",
            MotionKind::ZoomIn => "Zoom in",
            MotionKind::ZoomOut => "Zoom out",
            MotionKind::KenBurns => "Ken Burns",
            MotionKind::Fade => "Fade",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PanDirection {
    /// Pick a direction per clip so consecutive clips vary.
    #[default]
    Auto,
    Left,
    Right,
    Up,
    Down,
}

impl PanDirection {
    pub const ALL: [PanDirection; 5] =
        [PanDirection::Auto, PanDirection::Left, PanDirection::Right, PanDirection::Up, PanDirection::Down];
    pub fn label(self) -> &'static str {
        match self {
            PanDirection::Auto => "Auto",
            PanDirection::Left => "Left",
            PanDirection::Right => "Right",
            PanDirection::Up => "Up",
            PanDirection::Down => "Down",
        }
    }
}

/// Frame animation applied to a picture while it is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub kind: MotionKind,
    /// 0.0 (subtle) ..= 1.0 (strong)
    pub intensity: f32,
    /// 0.25 (slow) ..= 3.0 (fast). 1.0 spreads the motion over the whole clip.
    pub speed: f32,
    pub direction: PanDirection,
}

impl Default for Motion {
    fn default() -> Self {
        Self { kind: MotionKind::KenBurns, intensity: 0.5, speed: 1.0, direction: PanDirection::Auto }
    }
}

/// A picture or video on the main track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Seconds on the timeline.
    pub duration: f64,
    /// For videos: where in the source file to start.
    #[serde(default)]
    pub source_in: f64,
    /// For videos: total length of the source file, if known.
    #[serde(default)]
    pub source_length: Option<f64>,
    #[serde(default)]
    pub motion: Motion,
    /// Transition from the previous clip into this one (ignored on the first clip).
    #[serde(default)]
    pub transition_in: Transition,
    #[serde(default)]
    pub fit: Fit,
    /// When true, auto-fit keeps this clip's duration.
    #[serde(default)]
    pub locked_duration: bool,
}

impl Clip {
    pub fn new(path: PathBuf, kind: MediaKind, duration: f64) -> Self {
        Self {
            id: new_id(),
            path,
            kind,
            duration,
            source_in: 0.0,
            source_length: None,
            motion: Motion::default(),
            transition_in: Transition::default(),
            fit: Fit::default(),
            locked_duration: false,
        }
    }
    pub fn name(&self) -> String {
        file_name(&self.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextAnim {
    None,
    #[default]
    Fade,
    Blur,
    RevealLeft,
    RevealRight,
    RevealTop,
    RevealBottom,
}

impl TextAnim {
    pub const ALL: [TextAnim; 7] = [
        TextAnim::None,
        TextAnim::Fade,
        TextAnim::Blur,
        TextAnim::RevealLeft,
        TextAnim::RevealRight,
        TextAnim::RevealTop,
        TextAnim::RevealBottom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            TextAnim::None => "None",
            TextAnim::Fade => "Fade",
            TextAnim::Blur => "Blur",
            TextAnim::RevealLeft => "Reveal from left",
            TextAnim::RevealRight => "Reveal from right",
            TextAnim::RevealTop => "Reveal from top",
            TextAnim::RevealBottom => "Reveal from bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// A text title on the text track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextClip {
    pub id: u64,
    pub text: String,
    /// Font family name; "Default" uses the bundled font.
    pub font: String,
    /// Text height as a fraction of the frame height.
    pub size: f32,
    pub color: [u8; 4],
    /// Optional shadow for legibility over pictures.
    #[serde(default = "default_true")]
    pub shadow: bool,
    #[serde(default)]
    pub align: TextAlign,
    /// Centre of the text box, 0..1 in frame coordinates.
    pub pos: [f32; 2],
    pub start: f64,
    pub duration: f64,
    pub anim_in: TextAnim,
    pub anim_out: TextAnim,
    /// Seconds each animation takes.
    pub anim_duration: f64,
}

fn default_true() -> bool {
    true
}

impl TextClip {
    pub fn new(start: f64) -> Self {
        Self {
            id: new_id(),
            text: "Your title".into(),
            font: crate::text::DEFAULT_FONT.into(),
            size: 0.09,
            color: [255, 255, 255, 255],
            shadow: true,
            align: TextAlign::Center,
            pos: [0.5, 0.5],
            start,
            duration: 4.0,
            anim_in: TextAnim::Fade,
            anim_out: TextAnim::Fade,
            anim_duration: 0.8,
        }
    }
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

/// A piece of music on the music track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: u64,
    pub path: PathBuf,
    pub start: f64,
    pub duration: f64,
    #[serde(default)]
    pub source_in: f64,
    #[serde(default)]
    pub source_length: Option<f64>,
    pub volume: f32,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl AudioClip {
    pub fn new(path: PathBuf, start: f64, length: Option<f64>) -> Self {
        Self {
            id: new_id(),
            path,
            start,
            duration: length.unwrap_or(30.0),
            source_in: 0.0,
            source_length: length,
            volume: 1.0,
            fade_in: 0.5,
            fade_out: 2.0,
        }
    }
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
    pub fn name(&self) -> String {
        file_name(&self.path)
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// A whole slideshow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub background: [u8; 3],
    pub clips: Vec<Clip>,
    pub texts: Vec<TextClip>,
    pub music: Vec<AudioClip>,
    pub playback: PlaybackMode,
    /// When set, auto-fit spaces the pictures to fill exactly this many seconds.
    pub time_limit: Option<f64>,
    /// Duration given to new pictures when no time limit is set.
    pub default_duration: f64,
    pub default_transition: Transition,
    pub default_motion: Motion,
    /// Fade in from the background at the very start.
    pub fade_in_start: bool,
    /// Fade out to the background at the very end.
    pub fade_out_end: bool,
    /// Trim or loop music to match the slideshow length.
    #[serde(default = "default_true")]
    pub fit_music: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            background: [0, 0, 0],
            clips: vec![],
            texts: vec![],
            music: vec![],
            playback: PlaybackMode::Loop,
            time_limit: None,
            default_duration: 4.0,
            default_transition: Transition::default(),
            default_motion: Motion::default(),
            fade_in_start: true,
            fade_out_end: true,
            fit_music: true,
        }
    }
}

impl Project {
    /// Length of the slideshow in seconds (the main track decides; text and music past the end are cut).
    pub fn duration(&self) -> f64 {
        let main: f64 = self.clips.iter().map(|c| c.duration).sum();
        if main > 0.0 {
            main
        } else {
            self.texts.iter().map(|t| t.end()).fold(0.0, f64::max)
        }
    }

    /// Start time of each clip on the main track.
    pub fn clip_starts(&self) -> Vec<f64> {
        let mut t = 0.0;
        self.clips
            .iter()
            .map(|c| {
                let s = t;
                t += c.duration;
                s
            })
            .collect()
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        let p: Project = serde_json::from_str(s)?;
        let max = p
            .clips
            .iter()
            .map(|c| c.id)
            .chain(p.texts.iter().map(|t| t.id))
            .chain(p.music.iter().map(|m| m.id))
            .max()
            .unwrap_or(0);
        bump_ids_past(max);
        Ok(p)
    }
}
