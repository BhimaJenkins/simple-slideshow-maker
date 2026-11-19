//! Auto-fit: spacing pictures, titles and music to fill a time limit.

use crate::model::{MediaKind, Project};

/// Shortest time a picture is allowed to get from auto-fit.
pub const MIN_CLIP: f64 = 0.5;

/// Effective transition length at the start of clip `i`, clamped so a transition
/// never eats more than ~45% of either neighbouring clip.
pub fn transition_len(p: &Project, i: usize) -> f64 {
    if i == 0 || i >= p.clips.len() {
        return 0.0;
    }
    let t = &p.clips[i].transition_in;
    if t.kind == crate::model::TransitionKind::Cut {
        return 0.0;
    }
    t.duration.max(0.0).min(0.45 * p.clips[i - 1].duration).min(0.45 * p.clips[i].duration)
}

/// Re-space the main track to fill `project.time_limit` exactly, then keep
/// titles and music in step. Does nothing without a time limit.
pub fn auto_fit(p: &mut Project) {
    let Some(limit) = p.time_limit else {
        fit_music(p);
        return;
    };
    let old_len = p.duration();
    if p.clips.is_empty() || limit <= 0.0 {
        return;
    }

    // Locked clips keep their length; videos never stretch past their source length.
    let locked: f64 = p.clips.iter().filter(|c| c.locked_duration).map(|c| c.duration).sum();
    let mut free: Vec<usize> = (0..p.clips.len()).filter(|&i| !p.clips[i].locked_duration).collect();
    let mut remaining = limit - locked;

    if free.is_empty() || remaining < MIN_CLIP * free.len() as f64 {
        // Not enough room: scale everything evenly instead.
        let total: f64 = p.clips.iter().map(|c| c.duration).sum();
        let k = limit / total.max(1e-6);
        for c in &mut p.clips {
            c.duration = (c.duration * k).max(0.1);
        }
    } else {
        // Hand out equal shares, capping videos at their natural length and
        // redistributing what they leave behind.
        loop {
            let share = remaining / free.len() as f64;
            let capped: Vec<usize> = free
                .iter()
                .copied()
                .filter(|&i| {
                    let c = &p.clips[i];
                    c.kind == MediaKind::Video && c.source_length.is_some_and(|l| l - c.source_in < share)
                })
                .collect();
            if capped.is_empty() || capped.len() == free.len() {
                for &i in &free {
                    let c = &mut p.clips[i];
                    c.duration = match (c.kind, c.source_length) {
                        (MediaKind::Video, Some(l)) if capped.len() == free.len() => (l - c.source_in).max(0.1),
                        _ => share,
                    };
                }
                break;
            }
            for &i in &capped {
                let c = &mut p.clips[i];
                c.duration = (c.source_length.unwrap() - c.source_in).max(0.1);
                remaining -= c.duration;
            }
            free.retain(|i| !capped.contains(i));
        }
    }

    // Keep titles in the same relative place.
    let new_len = p.duration();
    if old_len > 0.0 && (new_len - old_len).abs() > 1e-6 {
        let k = new_len / old_len;
        for t in &mut p.texts {
            t.start *= k;
            t.duration = (t.duration * k).max(0.5);
        }
    }
    clamp_texts(p);
    fit_music(p);
}

/// Keep titles inside the slideshow.
pub fn clamp_texts(p: &mut Project) {
    let len = p.duration();
    if len <= 0.0 {
        return;
    }
    for t in &mut p.texts {
        t.duration = t.duration.min(len).max(0.2);
        t.start = t.start.clamp(0.0, (len - t.duration).max(0.0));
    }
}

/// Trim music so it ends with the slideshow (when `fit_music` is on).
pub fn fit_music(p: &mut Project) {
    if !p.fit_music || p.music.is_empty() {
        return;
    }
    let len = p.duration();
    if len <= 0.0 {
        return;
    }
    p.music.sort_by(|a, b| a.start.total_cmp(&b.start));
    p.music.retain(|m| m.start < len);
    if let Some(last) = p.music.last_mut() {
        let avail = last.source_length.map(|l| l - last.source_in).unwrap_or(f64::MAX);
        last.duration = (len - last.start).min(avail).max(0.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::path::PathBuf;

    fn img(d: f64) -> Clip {
        Clip::new(PathBuf::from("a.png"), MediaKind::Image, d)
    }

    #[test]
    fn fits_images_evenly() {
        let mut p = Project { time_limit: Some(30.0), ..Default::default() };
        p.clips = vec![img(4.0), img(4.0), img(4.0)];
        auto_fit(&mut p);
        assert!((p.duration() - 30.0).abs() < 1e-9);
        assert!(p.clips.iter().all(|c| (c.duration - 10.0).abs() < 1e-9));
    }

    #[test]
    fn short_videos_keep_their_length() {
        let mut p = Project { time_limit: Some(20.0), ..Default::default() };
        let mut v = Clip::new(PathBuf::from("v.mp4"), MediaKind::Video, 3.0);
        v.source_length = Some(2.0);
        p.clips = vec![img(1.0), v, img(1.0)];
        auto_fit(&mut p);
        assert!((p.clips[1].duration - 2.0).abs() < 1e-9);
        assert!((p.clips[0].duration - 9.0).abs() < 1e-9);
        assert!((p.duration() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn locked_clips_are_kept() {
        let mut p = Project { time_limit: Some(10.0), ..Default::default() };
        let mut a = img(6.0);
        a.locked_duration = true;
        p.clips = vec![a, img(1.0), img(1.0)];
        auto_fit(&mut p);
        assert_eq!(p.clips[0].duration, 6.0);
        assert!((p.clips[1].duration - 2.0).abs() < 1e-9);
    }

    #[test]
    fn titles_scale_and_music_trims() {
        let mut p = Project { time_limit: Some(20.0), ..Default::default() };
        p.clips = vec![img(5.0), img(5.0)];
        let mut t = TextClip::new(5.0);
        t.duration = 2.0;
        p.texts.push(t);
        p.music.push(AudioClip::new(PathBuf::from("m.mp3"), 0.0, Some(120.0)));
        auto_fit(&mut p);
        assert!((p.texts[0].start - 10.0).abs() < 1e-9);
        assert!((p.music[0].duration - 20.0).abs() < 1e-9);
    }

    #[test]
    fn transitions_are_clamped() {
        let mut p = Project::default();
        let mut b = img(1.0);
        b.transition_in.duration = 5.0;
        p.clips = vec![img(4.0), b];
        assert!((transition_len(&p, 1) - 0.45).abs() < 1e-9);
        assert_eq!(transition_len(&p, 0), 0.0);
    }
}
