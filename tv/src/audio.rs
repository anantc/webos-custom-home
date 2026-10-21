use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

const SAMPLE_RATE: i32 = 48_000;
const CLICK_MS: usize = 40;
const MIN_INTERVAL: Duration = Duration::from_millis(60);

/// Original, softly damped click; generated once, with no asset decoding at input time.
fn click_samples() -> Vec<f32> {
    let count = SAMPLE_RATE as usize * CLICK_MS / 1000;
    let mut noise = 0x6d2b_79f5u32;
    let mut filtered_noise = 0.0f32;
    (0..count)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            let white = noise as f32 / u32::MAX as f32 * 2.0 - 1.0;
            filtered_noise += 0.28 * (white - filtered_noise);
            let attack = (t / 0.0015).min(1.0);
            let tail = ((count - 1 - i) as f32 / (SAMPLE_RATE as f32 * 0.006)).min(1.0);
            let body = (TAU * 820.0 * t).sin() * (-t / 0.007).exp();
            let overtone = (TAU * 1640.0 * t).sin() * (-t / 0.004).exp();
            let texture = filtered_noise * (-t / 0.003).exp();
            0.30 * attack * tail * (0.70 * body + 0.18 * overtone + 0.12 * texture)
        })
        .collect()
}

#[derive(Default)]
struct ClickGate {
    last: Option<Instant>,
}

impl ClickGate {
    fn accept(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < MIN_INTERVAL)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

struct Playback {
    // Drop the stream before its owning device. No Rust audio callback is installed.
    stream: AudioStreamOwner,
    _device: AudioDevice,
}

impl Playback {
    fn open(sdl: &sdl3::Sdl) -> Result<Self, sdl3::Error> {
        sdl3::hint::set("SDL_AUDIO_DEVICE_STREAM_NAME", "Home navigation");
        sdl3::hint::set("SDL_AUDIO_DEVICE_SAMPLE_FRAMES", "512");
        let audio = sdl.audio()?;
        let spec = AudioSpec {
            freq: Some(SAMPLE_RATE),
            channels: Some(1),
            format: Some(AudioFormat::f32_sys()),
        };
        let device = audio.open_playback_device(&spec)?;
        let stream = audio.new_stream(Some(&spec), None)?;
        device.bind_stream(&stream)?;
        stream.pause()?;
        tracing::info!(
            driver = audio.current_audio_driver(),
            "navigation audio ready"
        );
        Ok(Self {
            stream,
            _device: device,
        })
    }
}

pub struct NavigationSound {
    playback: Option<Playback>,
    samples: Vec<f32>,
    gate: ClickGate,
}

impl NavigationSound {
    pub fn new(sdl: &sdl3::Sdl) -> Self {
        let playback = match Playback::open(sdl) {
            Ok(playback) => Some(playback),
            Err(error) => {
                tracing::warn!(%error, "navigation audio unavailable; continuing silently");
                None
            }
        };
        Self {
            playback,
            samples: click_samples(),
            gate: ClickGate::default(),
        }
    }

    pub fn play(&mut self) {
        let Some(playback) = &self.playback else {
            return;
        };
        if !self.gate.accept(Instant::now()) {
            return;
        }
        // Keep only the newest click if the output device falls behind. SDL plays it
        // asynchronously; input never waits for playback, decoding or a subprocess.
        let result = playback
            .stream
            .clear()
            .and_then(|_| playback.stream.put_data_f32(&self.samples))
            .and_then(|_| playback.stream.flush())
            .and_then(|_| playback.stream.resume());
        if let Err(error) = result {
            tracing::warn!(%error, "navigation audio stopped; continuing silently");
            self.playback = None;
        }
    }

    pub fn suspend(&mut self) {
        self.gate = ClickGate::default();
        if let Some(playback) = &self.playback {
            let result = playback
                .stream
                .pause()
                .and_then(|_| playback.stream.clear());
            if let Err(error) = result {
                tracing::warn!(%error, "navigation audio stopped; continuing silently");
                self.playback = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_is_bounded_and_has_silent_edges() {
        let samples = click_samples();
        assert_eq!(samples.len(), 1920);
        assert_eq!(samples.first(), Some(&0.0));
        assert_eq!(samples.last(), Some(&0.0));
        assert!(samples.iter().all(|v| v.is_finite() && v.abs() <= 0.30));
        assert!(samples.iter().any(|v| v.abs() > 0.04));
    }

    #[test]
    fn rapid_changes_do_not_queue_clicks_or_extend_the_cooldown() {
        let mut gate = ClickGate::default();
        let start = Instant::now();
        assert!(gate.accept(start));
        assert!(!gate.accept(start + Duration::from_millis(30)));
        assert!(!gate.accept(start + Duration::from_millis(59)));
        assert!(gate.accept(start + Duration::from_millis(60)));
    }

    #[test]
    fn playback_resumes_and_discards_pending_audio_on_suspend() {
        // The preview container has no speaker. Exercise SDL's real stream lifecycle.
        sdl3::hint::set("SDL_AUDIO_DRIVER", "dummy");
        let sdl = sdl3::init().unwrap();
        let mut sound = NavigationSound::new(&sdl);
        assert!(sound
            .playback
            .as_ref()
            .unwrap()
            .stream
            .device_paused()
            .unwrap());
        sound.play();
        assert!(!sound
            .playback
            .as_ref()
            .unwrap()
            .stream
            .device_paused()
            .unwrap());
        sound.suspend();
        let stream = &sound.playback.as_ref().unwrap().stream;
        assert!(stream.device_paused().unwrap());
        assert_eq!(stream.queued_bytes().unwrap(), 0);
        sound.play();
        assert!(!sound
            .playback
            .as_ref()
            .unwrap()
            .stream
            .device_paused()
            .unwrap());
    }
}
