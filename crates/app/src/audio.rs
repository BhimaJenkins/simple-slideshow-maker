//! Music playback for the live preview. Export mixes audio with FFmpeg instead.

use slideshow_core::Project;

#[cfg(feature = "audio-preview")]
mod imp {
    use super::*;
    use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
    use std::time::Duration;

    #[derive(Default)]
    pub struct AudioPlayer {
        device: Option<MixerDeviceSink>,
        players: Vec<Player>,
        failed: bool,
    }

    impl AudioPlayer {
        /// Start every music clip that is still to come, as of time `t`.
        pub fn play_from(&mut self, p: &Project, t: f64) {
            self.stop();
            if p.music.is_empty() || self.failed {
                return;
            }
            if self.device.is_none() {
                match DeviceSinkBuilder::open_default_sink() {
                    Ok(mut d) => {
                        d.log_on_drop(false);
                        self.device = Some(d);
                    }
                    Err(e) => {
                        log::warn!("no audio output: {e}");
                        self.failed = true;
                        return;
                    }
                }
            }
            let mixer = self.device.as_ref().unwrap().mixer();
            let end = p.duration();
            for m in &p.music {
                let clip_end = m.end().min(end);
                if clip_end <= t {
                    continue;
                }
                let Ok(file) = std::fs::File::open(&m.path) else { continue };
                let Ok(dec) = Decoder::try_from(file) else { continue };
                let into = (t - m.start).max(0.0);
                let wait = (m.start - t).max(0.0);
                let len = clip_end - m.start.max(t);
                let src = dec
                    .skip_duration(Duration::from_secs_f64(m.source_in.max(0.0) + into))
                    .take_duration(Duration::from_secs_f64(len))
                    .amplify(m.volume)
                    .delay(Duration::from_secs_f64(wait));
                let player = Player::connect_new(mixer);
                player.append(src);
                self.players.push(player);
            }
        }

        pub fn stop(&mut self) {
            for pl in self.players.drain(..) {
                pl.stop();
            }
        }
    }
}

#[cfg(not(feature = "audio-preview"))]
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct AudioPlayer;
    impl AudioPlayer {
        pub fn play_from(&mut self, _p: &Project, _t: f64) {}
        pub fn stop(&mut self) {}
    }
}

pub use imp::AudioPlayer;
