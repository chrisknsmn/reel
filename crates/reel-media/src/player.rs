//! Playback's clock, driven by the sound card when there is one (§37, BUILD_PLAN 3.7).
//!
//! The sound card can't be hurried or slowed, so video follows it. With no sound device, the same
//! clock runs on wall time. Either way, gaps between clips are skipped.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::Clip;

use crate::mix::{CHANNELS, Mixer, SAMPLE_RATE, frames_to_time};

/// Where the timeline's next playable moment is, skipping gaps. `None` means the end.
fn skip_gaps(clips: &[Clip], time: TimelineTime) -> Option<TimelineTime> {
    clips
        .iter()
        .find(|clip| clip.end() > time)
        .map(|clip| clip.start.max(time))
}

/// The clock itself, kept apart from any sound hardware so it can be tested.
#[derive(Debug)]
struct ClockState {
    playing: AtomicBool,
    /// Where the timeline was when `stamp` was taken, in microseconds.
    position: AtomicI64,
    /// How far ahead the sound card's buffer is of what's heard.
    latency: AtomicI64,
    /// When `position` was last updated, in microseconds since `epoch`.
    stamp: AtomicI64,
    epoch: Instant,
    /// Set when sound is really driving the clock.
    audio_driven: AtomicBool,
    clips: Vec<Clip>,
}

/// A handle any thread can read the time from.
#[derive(Clone, Debug)]
pub struct Clock {
    state: Arc<ClockState>,
}

/// How long the clock will keep running on its own between updates from the sound card, before
/// assuming it has stopped. Long enough for any normal buffer.
const LONGEST_COAST: Duration = Duration::from_millis(100);

impl Clock {
    #[must_use]
    pub fn new(clips: Vec<Clip>) -> Self {
        Self {
            state: Arc::new(ClockState {
                playing: AtomicBool::new(false),
                position: AtomicI64::new(0),
                latency: AtomicI64::new(0),
                stamp: AtomicI64::new(0),
                epoch: Instant::now(),
                audio_driven: AtomicBool::new(false),
                clips,
            }),
        }
    }

    fn since_epoch(&self, at: Instant) -> i64 {
        at.duration_since(self.state.epoch).as_micros() as i64
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.state.playing.load(Ordering::Relaxed)
    }

    /// Where playback is right now, at this instant.
    #[must_use]
    pub fn now(&self) -> TimelineTime {
        self.at(Instant::now())
    }

    fn at(&self, instant: Instant) -> TimelineTime {
        let state = &self.state;
        let position = state.position.load(Ordering::Relaxed);
        if !self.is_playing() {
            return TimelineTime(position);
        }
        let elapsed = (self.since_epoch(instant) - state.stamp.load(Ordering::Relaxed)).max(0);
        if state.audio_driven.load(Ordering::Relaxed) {
            // Between updates from the sound card, carry on at normal speed.
            let coast = elapsed.min(LONGEST_COAST.as_micros() as i64);
            return TimelineTime((position + coast - state.latency.load(Ordering::Relaxed)).max(0));
        }
        // On wall time. Gaps are skipped here, since no sound callback is doing it.
        let time = TimelineTime(position + elapsed);
        skip_gaps(&state.clips, time).unwrap_or_else(|| self.end())
    }

    fn end(&self) -> TimelineTime {
        self.state
            .clips
            .iter()
            .map(Clip::end)
            .max()
            .unwrap_or_default()
    }

    #[must_use]
    pub fn at_end(&self) -> bool {
        self.now() >= self.end()
    }

    pub fn play(&self, now: Instant) {
        let state = &self.state;
        if self.at_end() {
            state.position.store(0, Ordering::Relaxed);
        }
        state.stamp.store(self.since_epoch(now), Ordering::Relaxed);
        state.playing.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self, now: Instant) {
        let time = self.at(now);
        self.state.playing.store(false, Ordering::Relaxed);
        self.state.position.store(time.0, Ordering::Relaxed);
    }

    /// Moves playback. Anything past the end stops at the end, and a gap moves on to the next clip.
    pub fn seek(&self, time: TimelineTime, now: Instant) {
        let target = skip_gaps(&self.state.clips, time.max(TimelineTime::ZERO))
            .unwrap_or_else(|| self.end());
        self.state.position.store(target.0, Ordering::Relaxed);
        self.state
            .stamp
            .store(self.since_epoch(now), Ordering::Relaxed);
    }

    /// Called from the sound callback: this much has now gone to the sound card.
    fn update_from_audio(&self, position: TimelineTime, latency: Micros, now: Instant) {
        let state = &self.state;
        state.audio_driven.store(true, Ordering::Relaxed);
        state.position.store(position.0, Ordering::Relaxed);
        state.latency.store(latency.0, Ordering::Relaxed);
        state.stamp.store(self.since_epoch(now), Ordering::Relaxed);
    }
}

/// Plays a timeline's sound and owns its clock. It has to stay on the thread that made it, because
/// some sound systems don't allow a stream to move, but `clock()` can go anywhere.
pub struct Player {
    clock: Clock,
    mixer: Arc<Mutex<Mixer>>,
    volume: Arc<AtomicI64>,
    /// `None` when there's no sound device, in which case the clock runs on wall time.
    stream: Option<cpal::Stream>,
}

impl Player {
    /// Sets up playback, paused at the start.
    #[must_use]
    pub fn new(mixer: Mixer, clips: Vec<Clip>) -> Self {
        let clock = Clock::new(clips);
        let mixer = Arc::new(Mutex::new(mixer));
        let volume = Arc::new(AtomicI64::new(1000));
        let stream = open_stream(clock.clone(), mixer.clone(), volume.clone());
        if let Some(stream) = &stream {
            // A stream that won't start is the same as having none.
            let _ = stream.play();
        }
        Self {
            clock,
            mixer,
            volume,
            stream,
        }
    }

    #[must_use]
    pub fn clock(&self) -> Clock {
        self.clock.clone()
    }

    /// Whether sound is really playing, rather than the clock running on wall time.
    #[must_use]
    pub fn has_sound_device(&self) -> bool {
        self.stream.is_some()
    }

    pub fn play(&self) {
        let now = Instant::now();
        if self.clock.at_end()
            && let Ok(mut mixer) = self.mixer.lock()
        {
            mixer.seek(TimelineTime::ZERO);
        }
        self.clock.play(now);
    }

    pub fn pause(&self) {
        let now = Instant::now();
        self.clock.pause(now);
        let time = self.clock.now();
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.seek(time);
        }
    }

    pub fn seek(&self, time: TimelineTime) {
        let now = Instant::now();
        self.clock.seek(time, now);
        let target = self.clock.state.position.load(Ordering::Relaxed);
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.seek(TimelineTime(target));
        }
    }

    /// Sets the preview's volume, from 0 to 1. This isn't saved or exported (§17).
    pub fn set_volume(&self, volume: f64) {
        self.volume
            .store((volume.clamp(0.0, 1.0) * 1000.0) as i64, Ordering::Relaxed);
    }
}

fn open_stream(
    clock: Clock,
    mixer: Arc<Mutex<Mixer>>,
    volume: Arc<AtomicI64>,
) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let wanted = device.supported_output_configs().ok()?.find(|range| {
        range.channels() as usize == CHANNELS
            && range.sample_format() == cpal::SampleFormat::F32
            && range.min_sample_rate() <= SAMPLE_RATE
            && range.max_sample_rate() >= SAMPLE_RATE
    });
    let config: cpal::StreamConfig = match wanted {
        Some(range) => range.with_sample_rate(SAMPLE_RATE).into(),
        None => device.default_output_config().ok()?.into(),
    };
    let device_channels = usize::from(config.channels);
    let device_rate = config.sample_rate;
    let ratio = f64::from(SAMPLE_RATE) / f64::from(device_rate);
    let mut resampler = Resampler::new(ratio);
    let mut block = Vec::<f32>::new();

    device
        .build_output_stream(
            config,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                if !clock.is_playing() {
                    data.fill(0.0);
                    return;
                }
                let now = Instant::now();
                let Ok(mut mixer) = mixer.try_lock() else {
                    data.fill(0.0);
                    return;
                };
                let frames = data.len() / device_channels;
                block.resize(frames * CHANNELS, 0.0);
                resampler.fill(&mut mixer, &mut block);

                let level = volume.load(Ordering::Relaxed) as f32 / 1000.0;
                for (out, stereo) in data
                    .chunks_exact_mut(device_channels)
                    .zip(block.chunks_exact(CHANNELS))
                {
                    for (channel, sample) in out.iter_mut().enumerate() {
                        *sample = stereo[channel.min(CHANNELS - 1)] * level;
                    }
                }

                let timestamp = info.timestamp();
                let latency = timestamp.playback.duration_since(timestamp.callback);
                clock.update_from_audio(mixer.position(), Micros(latency.as_micros() as i64), now);
            },
            |error| eprintln!("Sound playback error: {error}"),
            None,
        )
        .ok()
}

/// Matches the mixer's rate to the sound card's, for cards that can't do the working rate.
struct Resampler {
    /// Mixer frames per output frame.
    ratio: f64,
    phase: f64,
    previous: [f32; 2],
    next: [f32; 2],
}

impl Resampler {
    fn new(ratio: f64) -> Self {
        Self {
            ratio,
            phase: 1.0,
            previous: [0.0; 2],
            next: [0.0; 2],
        }
    }

    fn fill(&mut self, mixer: &mut Mixer, out: &mut [f32]) {
        if (self.ratio - 1.0).abs() < 1e-9 {
            skip_gap(mixer);
            mixer.render(out);
            return;
        }
        let mut one = [0.0_f32; CHANNELS];
        for frame in out.chunks_exact_mut(CHANNELS) {
            while self.phase >= 1.0 {
                self.previous = self.next;
                skip_gap(mixer);
                mixer.render(&mut one);
                self.next = one;
                self.phase -= 1.0;
            }
            let t = self.phase as f32;
            frame[0] = self.previous[0] + (self.next[0] - self.previous[0]) * t;
            frame[1] = self.previous[1] + (self.next[1] - self.previous[1]) * t;
            self.phase += self.ratio;
        }
    }
}

/// Jumps over any gap the mixer has reached, so playback carries straight on (§37).
fn skip_gap(mixer: &mut Mixer) {
    let frame = crate::mix::timeline_frames(mixer.position() - TimelineTime::ZERO);
    if let Some(next) = mixer.next_playable(frame)
        && next != frame
    {
        mixer.seek(frames_to_time(next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::time::SourceTime;

    fn clip(start_ms: f64, length_ms: f64) -> Clip {
        Clip {
            id: "c".into(),
            start: TimelineTime::from_ms(start_ms),
            duration: Micros::from_ms(length_ms),
            source_start: SourceTime::ZERO,
            speed: 1.0,
            muted: false,
            show_source_audio: false,
            track_settings: Vec::new(),
        }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn a_paused_clock_stands_still() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        assert_eq!(clock.at(Instant::now() + ms(500)), TimelineTime::ZERO);
    }

    #[test]
    fn on_wall_time_it_runs_at_normal_speed() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let start = Instant::now();
        clock.play(start);
        assert_eq!(clock.at(start + ms(250)), TimelineTime::from_ms(250.0));
    }

    #[test]
    fn pausing_keeps_the_time_it_reached() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let start = Instant::now();
        clock.play(start);
        clock.pause(start + ms(1000));
        assert_eq!(clock.at(start + ms(5000)), TimelineTime::from_ms(1000.0));
        assert!(!clock.is_playing());
    }

    #[test]
    fn seeking_moves_the_clock_while_playing_and_paused() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let start = Instant::now();
        clock.seek(TimelineTime::from_ms(3000.0), start);
        assert_eq!(clock.at(start + ms(100)), TimelineTime::from_ms(3000.0));
        clock.play(start);
        assert_eq!(clock.at(start + ms(100)), TimelineTime::from_ms(3100.0));
    }

    #[test]
    fn wall_time_playback_skips_gaps() {
        let clock = Clock::new(vec![clip(0.0, 1000.0), clip(5000.0, 1000.0)]);
        let start = Instant::now();
        clock.play(start);
        assert_eq!(clock.at(start + ms(2000)), TimelineTime::from_ms(5000.0));
    }

    #[test]
    fn seeking_into_a_gap_lands_on_the_next_clip() {
        let clock = Clock::new(vec![clip(0.0, 1000.0), clip(5000.0, 1000.0)]);
        let now = Instant::now();
        clock.seek(TimelineTime::from_ms(2000.0), now);
        assert_eq!(clock.at(now), TimelineTime::from_ms(5000.0));
    }

    #[test]
    fn seeking_past_the_end_stops_at_the_end() {
        let clock = Clock::new(vec![clip(0.0, 1000.0)]);
        let now = Instant::now();
        clock.seek(TimelineTime::from_ms(99_000.0), now);
        assert_eq!(clock.at(now), TimelineTime::from_ms(1000.0));
        assert!(clock.at_end());
    }

    #[test]
    fn playing_from_the_end_starts_over() {
        let clock = Clock::new(vec![clip(0.0, 1000.0)]);
        let now = Instant::now();
        clock.seek(TimelineTime::from_ms(1000.0), now);
        clock.play(Instant::now());
        assert!(clock.now() < TimelineTime::from_ms(100.0));
    }

    #[test]
    fn sound_drives_the_clock_minus_what_is_still_in_the_buffer() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let now = Instant::now();
        clock.play(now);
        clock.update_from_audio(TimelineTime::from_ms(2000.0), Micros::from_ms(30.0), now);
        assert_eq!(clock.at(now), TimelineTime::from_ms(1970.0));
    }

    #[test]
    fn between_sound_updates_the_clock_keeps_moving() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let now = Instant::now();
        clock.play(now);
        clock.update_from_audio(TimelineTime::from_ms(2000.0), Micros::ZERO, now);
        assert_eq!(clock.at(now + ms(5)), TimelineTime::from_ms(2005.0));
    }

    #[test]
    fn if_sound_stops_coming_the_clock_doesnt_run_away() {
        let clock = Clock::new(vec![clip(0.0, 10_000.0)]);
        let now = Instant::now();
        clock.play(now);
        clock.update_from_audio(TimelineTime::from_ms(2000.0), Micros::ZERO, now);
        assert_eq!(clock.at(now + ms(5000)), TimelineTime::from_ms(2100.0));
    }

    #[test]
    fn the_resampler_matches_a_different_sound_card_rate() {
        use crate::mix::{Pcm, SourceAudio};
        use reel_core::project::{Project, SourceTrackKind};
        use reel_core::timeline::Timeline;
        // A 440 Hz tone, played through at 44.1 kHz.
        let frames = SAMPLE_RATE as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let value = ((frame as f64 / f64::from(SAMPLE_RATE) * 440.0 * std::f64::consts::TAU)
                .sin()
                * 16000.0) as i16;
            samples.extend_from_slice(&[value, value]);
        }
        let project = Project {
            timeline: Timeline::for_recording(Micros::from_secs(1)),
            ..Project::default()
        };
        let source = SourceAudio {
            kind: SourceTrackKind::Mixed,
            pcm: Pcm::new(samples),
            volume: 1.0,
            boost: false,
        };
        let mut mixer = Mixer::new(&project, vec![source], vec![]);
        let mut resampler = Resampler::new(48_000.0 / 44_100.0);
        let mut out = vec![0.0; 44_100 / 2 * CHANNELS];
        resampler.fill(&mut mixer, &mut out);
        let left: Vec<f32> = out.iter().step_by(CHANNELS).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let measured = crossings as f64 / 0.5;
        assert!((measured - 440.0).abs() < 6.0, "{measured} Hz");
    }
}
