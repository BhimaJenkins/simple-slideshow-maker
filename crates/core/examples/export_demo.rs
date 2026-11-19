//! Build a project from files on the command line and export it, without the UI.
//!
//! cargo run --release -p slideshow-core --example export_demo -- out.mp4 a.jpg b.png clip.mp4 song.mp3
//! (use an `.gif` output name for a GIF; add `--limit 20` to fit into 20 seconds)

use slideshow_core::export::{export, ExportFormat, ExportSettings};
use slideshow_core::*;
use std::path::PathBuf;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut limit = None;
    if let Some(i) = args.iter().position(|a| a == "--limit") {
        limit = args.get(i + 1).and_then(|s| s.parse().ok());
        args.drain(i..(i + 2).min(args.len()));
    }
    let out = PathBuf::from(args.first().expect("usage: export_demo OUT FILES..."));
    let mut p = Project { time_limit: limit, ..Default::default() };
    let kinds = [TransitionKind::Crossfade, TransitionKind::DipToBlack, TransitionKind::DipToWhite, TransitionKind::FadeInOut];
    for (i, f) in args[1..].iter().enumerate() {
        let path = PathBuf::from(f);
        match media::kind_of(&path) {
            Some(MediaKind::Audio) => p.music.push(AudioClip::new(path.clone(), 0.0, media::probe_duration(&path))),
            Some(kind) => {
                let len = media::probe_duration(&path).filter(|_| kind == MediaKind::Video);
                let mut c = Clip::new(path, kind, len.unwrap_or(3.0));
                c.source_length = len;
                c.transition_in.kind = kinds[i % kinds.len()];
                if kind == MediaKind::Video {
                    c.motion.kind = MotionKind::None;
                }
                p.clips.push(c);
            }
            None => eprintln!("skipping {f}"),
        }
    }
    let mut title = TextClip::new(0.3);
    title.text = "Simple Slideshow Maker".into();
    title.anim_in = TextAnim::RevealLeft;
    title.anim_out = TextAnim::Blur;
    p.texts.push(title);
    timing::auto_fit(&mut p);

    let format = if out.extension().is_some_and(|e| e == "gif") { ExportFormat::Gif } else { ExportFormat::Mp4 };
    let mut s = ExportSettings::for_format(format);
    if format == ExportFormat::Mp4 {
        s.height = 720;
    }
    println!("{:.1}s, estimated {:.1} MB", p.duration(), s.estimate_bytes(&p) as f64 / 1048576.0);
    let t0 = std::time::Instant::now();
    let mut last = -1;
    match export(&p, &s, &out, &mut |f| {
        let pct = (f * 10.0) as i32;
        if pct != last {
            last = pct;
            print!("{}% ", pct * 10);
        }
        true
    }) {
        Ok(path) => println!("\nwrote {} ({} KB) in {:.1}s", path.display(), std::fs::metadata(&path).map(|m| m.len() / 1024).unwrap_or(0), t0.elapsed().as_secs_f32()),
        Err(e) => {
            eprintln!("\nexport failed: {e}");
            std::process::exit(1);
        }
    }
}
