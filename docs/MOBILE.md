# Android and iOS

The editor is written so it can run on phones and tablets: `slideshow-core` has no UI or OS code, and
the interface is egui, which runs on Android and iOS. What is left for a store-ready mobile build:

## Android

1. Install the Android SDK + NDK and `cargo install cargo-ndk` (or `xbuild`).
2. `cargo ndk -t arm64-v8a -P 26 build --release -p simple-slideshow-maker --lib` builds `libsimple_slideshow_maker.so`
   with the `android_main` entry point (NativeActivity).
3. Package it with a small Gradle project or `xbuild` (`x build --platform android`).

To do:
- File picking through the Storage Access Framework (desktop uses `rfd`, which has no Android backend).
- Bundle FFmpeg (for example the FFmpegKit `.so` libraries) and call it through `SSM_FFMPEG`, or
  switch video decode/encode to Android's MediaCodec. Until then, pictures, music preview and GIF export work.
- Touch tuning: larger timeline handles and a stacked layout on narrow screens.

## iOS

1. `rustup target add aarch64-apple-ios`.
2. Build the UI with eframe's `wgpu` (Metal) backend and wrap it in a minimal Xcode app (or use `xbuild`).

To do: the same file-picking (UIDocumentPicker / Photos) and FFmpeg/AVFoundation work as Android.
