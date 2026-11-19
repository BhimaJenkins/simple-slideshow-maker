# Simple Slideshow Maker

A simple, modern slideshow creator written in Rust. Drop in pictures, video clips and music, pick a
look, and export an MP4 or an animated GIF. The interface is a pared-down, Premiere-style editor:
media bin on the left, preview in the middle, settings on the right and a three-track timeline below.

## Features

- **Timeline** with three tracks: **Media** (pictures and video), **Text** and **Music**.
  Drag clips to reorder, drag a clip's right edge to change its length, click the ruler to move the playhead.
- **Transitions** between clips: fade in/out, cross fade, dip to black, dip to white (or a hard cut),
  with adjustable length. Optional fade in at the start and fade out at the end of the show.
- **Frame animation** per picture: Ken Burns (pan + zoom), zoom in, zoom out, or fade, each with
  **intensity** and **speed** controls and a pan direction for Ken Burns.
- **Titles**: type text, drag it into place in the preview, choose any installed font, size, colour,
  shadow and alignment. Animate it in and out with fade, blur, or a mask reveal from the left, right,
  top or bottom.
- **Loop or play once**, for preview and for GIF output.
- **Fit to a time limit**: turn on *Fit to* in the toolbar and set the length. Every picture you drop
  in is spaced evenly to fill exactly that time, titles keep their relative place and the music is
  trimmed to end with the show. Lock a clip to keep its length while the rest re-flow.
- **Music track** with start offset, trim, volume and fade in/out, played live in the preview.
- **Export** to **MP4** (H.264 + AAC, with music) or **animated GIF** (no sound), with simple size
  options: resolution, frame rate, *Small / Balanced / High* quality, an optional target file size
  for MP4, and a size estimate before you export.
- Undo/redo, save/open projects (`.ssm`), and the last project is restored on launch.

**Accepted files**

| Type     | Formats |
|----------|---------|
| Pictures | JPEG, PNG, GIF, WebP, BMP, TIFF, TGA, ICO, PNM, and HEIC/HEIF/AVIF (via FFmpeg) |
| Video    | MP4, MOV, M4V, MKV, WebM, AVI, WMV, MPEG, 3GP, FLV, MTS/M2TS, OGV (via FFmpeg) |
| Music    | MP3, WAV, FLAC, OGG, Opus, M4A/AAC, AIFF, WMA |

## Running it

You need [Rust](https://rustup.rs) (stable). For video clips and MP4 export, install
[FFmpeg](https://ffmpeg.org/download.html) so `ffmpeg` and `ffprobe` are on your PATH, or place them
next to the app, or point `SSM_FFMPEG` / `SSM_FFPROBE` at them. Pictures, music preview and GIF export
work without FFmpeg.

```sh
cargo run --release
# or open files straight away
cargo run --release -- photo1.jpg photo2.png song.mp3
```

Linux needs a few system packages to build: `sudo apt install libasound2-dev libxkbcommon-dev`
(Debian/Ubuntu).

**Shortcuts:** Space play/pause · Home back to start · ←/→ step one frame · Delete remove the
selected item · Ctrl/Cmd+Z undo · Ctrl/Cmd+Shift+Z redo · Ctrl/Cmd+S save.

## Platforms

| Platform | Status |
|----------|--------|
| Windows, macOS, Linux | Working. Built and tested in CI. |
| Android | The app builds as an Android library (`android_main` entry point); packaging and a file picker are next. See [docs/MOBILE.md](docs/MOBILE.md). |
| iOS | Planned: the core and UI are platform-neutral, packaging is next. See [docs/MOBILE.md](docs/MOBILE.md). |

## How it's built

```
crates/
  core/   slideshow-core: project model, auto-fit timing, CPU frame renderer,
          text rendering, MP4 (FFmpeg) and GIF (pure Rust) exporters. No UI.
  app/    simple-slideshow-maker: the egui/eframe interface.
```

- The **same renderer** draws the live preview (small, on a background thread) and the exported
  frames (full size), so what you see is what you export.
- **Pictures** are decoded in pure Rust (`image`), so they work everywhere. **Video** frames are
  streamed out of FFmpeg; **MP4** export pipes rendered frames into FFmpeg and mixes the music with
  an FFmpeg filter graph. **GIF** export uses the pure-Rust `gif` encoder, so it works on every platform.
- Live **music preview** uses `rodio`.

Try the renderer without the UI:

```sh
cargo run --release -p slideshow-core --example export_demo -- out.mp4 a.jpg b.png clip.mp4 song.mp3 --limit 20
```

Run the tests with `cargo test --workspace`.

## License

MIT
