//! Turns a timeline into sound. Preview and export both use this, so they can't disagree (§37, §42).
//!
//! Sound is rendered on the timeline's clock: a clip at 2× has already been sped up by the time it
//! comes out of here, so one output sample is always one sample of timeline time. That's what makes
//! it usable as the master clock for playback.

use std::sync::Arc;

use reel_core::project::{Project, SourceTrack, SourceTrackKind, TrackSetting};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::Clip;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Recordly calls this "Normalize", but it's a fixed gain (BUILD_PLAN 7.6).
pub const BOOST: f32 = 1.35;

/// Stereo sound at the working sample rate, ready for random access.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pcm {
    pub samples: Arc<Vec<i16>>,
}

impl Pcm {
    #[must_use]
    pub fn new(samples: Vec<i16>) -> Self {
        Self {
            samples: Arc::new(samples),
        }
    }

    #[must_use]
    pub fn frames(&self) -> i64 {
        (self.samples.len() / CHANNELS) as i64
    }

    fn frame(&self, index: i64) -> [f32; 2] {
        if index < 0 || index >= self.frames() {
            return [0.0, 0.0];
        }
        let at = index as usize * CHANNELS;
        [
            f32::from(self.samples[at]) / 32768.0,
            f32::from(self.samples[at + 1]) / 32768.0,
        ]
    }
}

fn micros_to_frames(micros: i64) -> i64 {
    (micros as f64 * f64::from(SAMPLE_RATE) / 1_000_000.0).round() as i64
}

/// Timeline time for a sample position.
#[must_use]
pub fn frames_to_time(frames: i64) -> TimelineTime {
    TimelineTime((frames as f64 * 1_000_000.0 / f64::from(SAMPLE_RATE)).round() as i64)
}

/// A quiet limiter, so a boost can't clip: nothing changes until close to full scale.
#[must_use]
pub fn soft_limit(sample: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let magnitude = sample.abs();
    if magnitude <= KNEE {
        return sample;
    }
    sample.signum() * (KNEE + (1.0 - KNEE) * ((magnitude - KNEE) / (1.0 - KNEE)).tanh())
}

/// One of the recording's audio tracks, ready to play.
#[derive(Clone, Debug)]
pub struct SourceAudio {
    pub kind: SourceTrackKind,
    pub pcm: Pcm,
    pub volume: f32,
    pub boost: bool,
}

/// An added audio item, ready to play. It always plays from the start of its file (§16).
#[derive(Clone, Debug)]
pub struct AddedAudio {
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub pcm: Pcm,
    pub volume: f32,
    pub boost: bool,
}

fn gain(volume: f32, boost: bool) -> f32 {
    volume * if boost { BOOST } else { 1.0 }
}

/// How loud a source track is in a clip. A clip's own setting wins (§58).
fn clip_gain(clip: &Clip, source: &SourceAudio) -> f32 {
    if clip.muted {
        return 0.0;
    }
    let own: Option<&TrackSetting> = clip
        .track_settings
        .iter()
        .find(|setting| setting.kind == source.kind);
    match own {
        Some(setting) => gain(setting.volume as f32, setting.boost),
        None => gain(source.volume, source.boost),
    }
}

// ---- Changing speed without changing pitch ----

/// How much sound each grain covers, about 43 ms.
const GRAIN: usize = 2048;
const HOP: usize = GRAIN / 2;
/// How far either way a grain can slide to line up with the sound before it, about 10 ms.
const SEARCH: i64 = 512;

fn hann(index: usize) -> f32 {
    let phase = index as f32 / GRAIN as f32;
    0.5 - 0.5 * (2.0 * std::f32::consts::PI * phase).cos()
}

/// WSOLA: overlaps grains of the original at the output's pace, reading them from further apart to
/// speed up. Each grain slides a little to line up with what's already playing, which avoids the
/// warble of a plain overlap-add.
#[derive(Clone, Debug)]
struct Stretcher {
    speed: f64,
    /// Where the next grain should ideally come from, in source frames.
    position: f64,
    /// Where the sound that would naturally have followed the last grain is.
    natural: Option<i64>,
    /// Sound still to come out, with the next grain's overlap already mixed in.
    overlap: Vec<[f32; 2]>,
    ready: std::collections::VecDeque<[f32; 2]>,
    /// The first hop only fades in, so it's dropped to keep timing exact.
    skip: usize,
}

impl Stretcher {
    fn new(source_frame: f64, speed: f64) -> Self {
        Self {
            speed,
            // Starting a hop early means the wanted position is at full volume, not fading in.
            position: source_frame - HOP as f64 * speed,
            natural: None,
            overlap: vec![[0.0; 2]; GRAIN],
            ready: std::collections::VecDeque::with_capacity(GRAIN),
            skip: HOP,
        }
    }

    fn best_offset(&self, pcm: &Pcm, ideal: i64) -> i64 {
        let Some(natural) = self.natural else {
            return ideal;
        };
        let mono = |index: i64| {
            let [left, right] = pcm.frame(index);
            left + right
        };
        let mut best = ideal;
        let mut best_score = f32::MIN;
        // Every 4th frame is plenty to line up the sound, and keeps this fast.
        for offset in (-SEARCH..=SEARCH).step_by(4) {
            let candidate = ideal + offset;
            let mut score = 0.0;
            let mut i = 0;
            while i < HOP as i64 {
                score += mono(candidate + i) * mono(natural + i);
                i += 8;
            }
            if score > best_score {
                best_score = score;
                best = candidate;
            }
        }
        best
    }

    fn make_grain(&mut self, pcm: &Pcm) {
        let start = self.best_offset(pcm, self.position.round() as i64);
        for (index, slot) in self.overlap.iter_mut().enumerate() {
            let [left, right] = pcm.frame(start + index as i64);
            let window = hann(index);
            slot[0] += left * window;
            slot[1] += right * window;
        }
        self.natural = Some(start + HOP as i64);
        self.position += HOP as f64 * self.speed;

        let finished: Vec<[f32; 2]> = self.overlap.drain(..HOP).collect();
        self.overlap.resize(GRAIN, [0.0; 2]);
        let drop = self.skip.min(finished.len());
        self.skip -= drop;
        self.ready.extend(&finished[drop..]);
    }

    fn pull(&mut self, pcm: &Pcm) -> [f32; 2] {
        while self.ready.is_empty() {
            self.make_grain(pcm);
        }
        self.ready.pop_front().unwrap_or([0.0; 2])
    }
}

// ---- The mixer ----

/// Renders a timeline's sound, moving forward from wherever it was last sent.
pub struct Mixer {
    clips: Vec<Clip>,
    sources: Vec<SourceAudio>,
    added: Vec<AddedAudio>,
    position: i64,
    /// One stretcher per source, for the clip they were started in.
    stretchers: Vec<Option<Stretcher>>,
    stretching_clip: Option<usize>,
}

impl Mixer {
    #[must_use]
    pub fn new(project: &Project, sources: Vec<SourceAudio>, added: Vec<AddedAudio>) -> Self {
        let count = sources.len();
        Self {
            clips: project.timeline.clips.clone(),
            sources,
            added,
            position: 0,
            stretchers: vec![None; count],
            stretching_clip: None,
        }
    }

    /// Where the next sample will be from.
    #[must_use]
    pub fn position(&self) -> TimelineTime {
        frames_to_time(self.position)
    }

    pub fn seek(&mut self, time: TimelineTime) {
        self.position = micros_to_frames(time.0);
        self.stretching_clip = None;
    }

    /// The next position anything is playing, which skips the gaps between clips (§37).
    #[must_use]
    pub fn next_playable(&self, frame: i64) -> Option<i64> {
        self.clips.iter().find_map(|clip| {
            let (start, end) = self.clip_frames(clip);
            (end > frame).then_some(start.max(frame))
        })
    }

    fn clip_frames(&self, clip: &Clip) -> (i64, i64) {
        let start = micros_to_frames(clip.start.0);
        (start, micros_to_frames(clip.end().0))
    }

    /// Fills a stereo buffer with the next stretch of timeline, moving forward by that much.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for frame in 0..out.len() / CHANNELS {
            let position = self.position + frame as i64;
            self.mix_clip_frame(position, &mut out[frame * CHANNELS..(frame + 1) * CHANNELS]);
        }
        self.mix_added(out);
        for sample in out.iter_mut() {
            *sample = soft_limit(*sample);
        }
        self.position += (out.len() / CHANNELS) as i64;
    }

    fn mix_clip_frame(&mut self, position: i64, out: &mut [f32]) {
        let found = self.clips.iter().position(|clip| {
            let (start, end) = self.clip_frames(clip);
            position >= start && position < end
        });
        let Some(index) = found else {
            self.stretching_clip = None;
            return;
        };
        let clip = self.clips[index].clone();
        let (clip_start, _) = self.clip_frames(&clip);
        let into_clip = position - clip_start;

        if (clip.speed - 1.0).abs() < 1e-9 {
            self.stretching_clip = None;
            let source_start = micros_to_frames(clip.source_start.0);
            for source in &self.sources {
                let [left, right] = source.pcm.frame(source_start + into_clip);
                let level = clip_gain(&clip, source);
                out[0] += left * level;
                out[1] += right * level;
            }
            return;
        }

        if self.stretching_clip != Some(index) {
            let source_frame =
                micros_to_frames(clip.source_start.0) as f64 + into_clip as f64 * clip.speed;
            self.stretchers =
                vec![Some(Stretcher::new(source_frame, clip.speed)); self.sources.len()];
            self.stretching_clip = Some(index);
        }
        for (source, stretcher) in self.sources.iter().zip(&mut self.stretchers) {
            let Some(stretcher) = stretcher else { continue };
            let [left, right] = stretcher.pull(&source.pcm);
            let level = clip_gain(&clip, source);
            out[0] += left * level;
            out[1] += right * level;
        }
    }

    fn mix_added(&self, out: &mut [f32]) {
        for item in &self.added {
            let (start, end) = (micros_to_frames(item.start.0), micros_to_frames(item.end.0));
            let level = gain(item.volume, item.boost);
            for frame in 0..out.len() / CHANNELS {
                let position = self.position + frame as i64;
                if position >= start && position < end {
                    let [left, right] = item.pcm.frame(position - start);
                    out[frame * CHANNELS] += left * level;
                    out[frame * CHANNELS + 1] += right * level;
                }
            }
        }
    }
}

/// How long the whole timeline is, in sound.
#[must_use]
pub fn timeline_frames(duration: Micros) -> i64 {
    micros_to_frames(duration.0)
}

/// Source tracks from a project, given a way to load a file's sound. Tracks with no file of their own
/// use the video's audio, which is passed in.
pub fn source_audio(
    tracks: &[SourceTrack],
    video_audio: Option<&Pcm>,
    mut load: impl FnMut(&SourceTrack) -> Option<Pcm>,
) -> Vec<SourceAudio> {
    let mut sources: Vec<SourceAudio> = tracks
        .iter()
        .filter_map(|track| {
            let pcm = if track.file.is_some() {
                load(track)
            } else {
                video_audio.cloned()
            }?;
            Some(SourceAudio {
                kind: track.kind,
                pcm,
                volume: track.volume as f32,
                boost: track.boost,
            })
        })
        .collect();
    if sources.is_empty()
        && let Some(pcm) = video_audio
    {
        sources.push(SourceAudio {
            kind: SourceTrackKind::Mixed,
            pcm: pcm.clone(),
            volume: 1.0,
            boost: false,
        });
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::edit::Edit;
    use reel_core::time::SourceTime;
    use reel_core::timeline::{ItemId, Timeline};

    fn tone(frequency: f64, seconds: f64) -> Pcm {
        let frames = (seconds * f64::from(SAMPLE_RATE)) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let t = frame as f64 / f64::from(SAMPLE_RATE);
            let value = ((t * frequency * std::f64::consts::TAU).sin() * 16000.0) as i16;
            samples.extend_from_slice(&[value, value]);
        }
        Pcm::new(samples)
    }

    fn project(seconds: i64) -> Project {
        Project {
            timeline: Timeline::for_recording(Micros::from_secs(seconds)),
            ..Project::default()
        }
    }

    fn source(pcm: Pcm) -> SourceAudio {
        SourceAudio {
            kind: SourceTrackKind::Mixed,
            pcm,
            volume: 1.0,
            boost: false,
        }
    }

    fn render(mixer: &mut Mixer, seconds: f64) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * f64::from(SAMPLE_RATE)) as usize * CHANNELS];
        mixer.render(&mut out);
        out
    }

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0, |peak: f32, sample| peak.max(sample.abs()))
    }

    /// Frequency from how often the left channel crosses zero.
    fn frequency(samples: &[f32]) -> f64 {
        let left: Vec<f32> = samples.iter().step_by(CHANNELS).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f64 / (left.len() as f64 / f64::from(SAMPLE_RATE))
    }

    #[test]
    fn a_plain_clip_plays_the_source_unchanged() {
        let pcm = tone(440.0, 1.0);
        let mut mixer = Mixer::new(&project(1), vec![source(pcm.clone())], vec![]);
        let out = render(&mut mixer, 0.5);
        for (index, sample) in out.iter().enumerate().step_by(97) {
            let expected = f32::from(pcm.samples[index]) / 32768.0;
            assert!((sample - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn a_clip_at_2x_lasts_half_as_long_at_the_same_pitch() {
        let mut project = project(4);
        project
            .timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 2.0,
            })
            .unwrap();
        assert_eq!(project.timeline.duration(), Micros::from_secs(2));
        let mut mixer = Mixer::new(&project, vec![source(tone(440.0, 4.0))], vec![]);
        let out = render(&mut mixer, 2.0);
        let measured = frequency(&out);
        assert!(
            (measured - 440.0).abs() < 8.0,
            "pitch moved to {measured} Hz"
        );
        assert!(peak(&out) > 0.3, "it should still be audible");
        let tail = render(&mut mixer, 0.2);
        assert_eq!(peak(&tail), 0.0, "silence after the clip ends");
    }

    #[test]
    fn a_clip_at_half_speed_keeps_its_pitch_too() {
        let mut project = project(2);
        project
            .timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 0.5,
            })
            .unwrap();
        let mut mixer = Mixer::new(&project, vec![source(tone(440.0, 2.0))], vec![]);
        let measured = frequency(&render(&mut mixer, 4.0));
        assert!(
            (measured - 440.0).abs() < 8.0,
            "pitch moved to {measured} Hz"
        );
    }

    #[test]
    fn sped_up_sound_stays_lined_up_with_the_video() {
        // A loud burst at 3 s of source, in a clip at 2×, should come out at 1.5 s.
        let frames = 6 * SAMPLE_RATE as usize;
        let mut samples = vec![0_i16; frames * 2];
        let burst = 3 * SAMPLE_RATE as usize;
        for frame in burst..burst + 4800 {
            let value = if (frame / 24) % 2 == 0 { 20000 } else { -20000 };
            samples[frame * 2] = value;
            samples[frame * 2 + 1] = value;
        }
        let mut project = project(6);
        project
            .timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 2.0,
            })
            .unwrap();
        let mut mixer = Mixer::new(&project, vec![source(Pcm::new(samples))], vec![]);
        let out = render(&mut mixer, 3.0);
        let first = out.iter().position(|sample| sample.abs() > 0.05).unwrap() / CHANNELS;
        let seconds = first as f64 / f64::from(SAMPLE_RATE);
        assert!(
            (seconds - 1.5).abs() < 0.03,
            "the burst came out at {seconds} s"
        );
    }

    #[test]
    fn a_moved_clip_plays_its_own_footages_sound() {
        // Two tones, and a clip that shows only the second, placed at the very start.
        let (first, second) = (tone(300.0, 2.0), tone(900.0, 2.0));
        let samples: Vec<i16> = first
            .samples
            .iter()
            .chain(second.samples.iter())
            .copied()
            .collect();
        let mut project = project(4);
        project.timeline.clips[0].source_start = SourceTime::from_ms(2000.0);
        project.timeline.clips[0].duration = Micros::from_secs(2);
        let mut mixer = Mixer::new(&project, vec![source(Pcm::new(samples))], vec![]);
        let measured = frequency(&render(&mut mixer, 1.0));
        assert!((measured - 900.0).abs() < 8.0, "got {measured} Hz");
    }

    #[test]
    fn a_muted_clip_is_silent_and_only_that_clip() {
        let mut project = project(2);
        project
            .timeline
            .apply(&Edit::SplitClip {
                at: TimelineTime::from_ms(1000.0),
            })
            .unwrap();
        project
            .timeline
            .apply(&Edit::SetClipMuted {
                id: "clip-1".into(),
                muted: true,
            })
            .unwrap();
        let mut mixer = Mixer::new(&project, vec![source(tone(440.0, 2.0))], vec![]);
        assert_eq!(peak(&render(&mut mixer, 1.0)), 0.0);
        assert!(peak(&render(&mut mixer, 1.0)) > 0.3);
    }

    #[test]
    fn muting_stays_correct_after_a_trim_moves_the_clip() {
        // §58: Recordly compares clip time with output time, so muted parts drift after trims.
        let mut project = project(4);
        project
            .timeline
            .apply(&Edit::SplitClip {
                at: TimelineTime::from_ms(2000.0),
            })
            .unwrap();
        let second: ItemId = project.timeline.clips[1].id.clone();
        project
            .timeline
            .apply(&Edit::SetClipMuted {
                id: second.clone(),
                muted: true,
            })
            .unwrap();
        project
            .timeline
            .apply(&Edit::MoveClip {
                id: second,
                to: TimelineTime::from_ms(2500.0),
            })
            .unwrap();
        let mut mixer = Mixer::new(&project, vec![source(tone(440.0, 4.0))], vec![]);
        assert!(peak(&render(&mut mixer, 2.0)) > 0.3, "the first clip plays");
        assert_eq!(
            peak(&render(&mut mixer, 2.5)),
            0.0,
            "the gap and the muted clip are silent"
        );
    }

    #[test]
    fn each_clip_uses_its_own_track_volume() {
        // §58: Recordly applies one clip's settings to the whole export.
        let mut project = project(2);
        project
            .timeline
            .apply(&Edit::SplitClip {
                at: TimelineTime::from_ms(1000.0),
            })
            .unwrap();
        project.timeline.clips[1].track_settings = vec![TrackSetting {
            kind: SourceTrackKind::Mixed,
            volume: 0.25,
            boost: false,
        }];
        let mut mixer = Mixer::new(&project, vec![source(tone(440.0, 2.0))], vec![]);
        let (first, second) = (
            peak(&render(&mut mixer, 1.0)),
            peak(&render(&mut mixer, 1.0)),
        );
        assert!(
            (second / first - 0.25).abs() < 0.01,
            "{first} then {second}"
        );
    }

    #[test]
    fn system_and_mic_tracks_are_both_heard() {
        // §58: Recordly's editor can lose the separately recorded mic.
        let system = SourceAudio {
            kind: SourceTrackKind::System,
            pcm: tone(300.0, 1.0),
            volume: 1.0,
            boost: false,
        };
        let mic = SourceAudio {
            kind: SourceTrackKind::Mic,
            pcm: tone(900.0, 1.0),
            volume: 1.0,
            boost: false,
        };
        let mut both = Mixer::new(&project(1), vec![system.clone(), mic], vec![]);
        let mut only = Mixer::new(&project(1), vec![system], vec![]);
        let (a, b) = (render(&mut both, 0.5), render(&mut only, 0.5));
        assert!(
            a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 0.1),
            "the mic should change the mix"
        );
    }

    #[test]
    fn boost_is_the_documented_gain() {
        let boosted = SourceAudio {
            kind: SourceTrackKind::Mixed,
            pcm: tone(440.0, 1.0),
            volume: 0.5,
            boost: true,
        };
        let mut mixer = Mixer::new(&project(1), vec![boosted], vec![]);
        let measured = peak(&render(&mut mixer, 0.5));
        let expected = 16000.0 / 32768.0 * 0.5 * 1.35;
        assert!((measured - expected).abs() < 0.01, "{measured}");
    }

    #[test]
    fn nothing_goes_past_full_scale() {
        let loud = SourceAudio {
            kind: SourceTrackKind::Mixed,
            pcm: tone(440.0, 1.0),
            volume: 3.0,
            boost: true,
        };
        let mut mixer = Mixer::new(&project(1), vec![loud], vec![]);
        assert!(peak(&render(&mut mixer, 0.5)) <= 1.0);
    }

    #[test]
    fn the_limiter_leaves_normal_levels_alone() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-0.9), -0.9);
        assert!(soft_limit(5.0) <= 1.0);
        assert!(soft_limit(-5.0) >= -1.0);
    }

    #[test]
    fn added_audio_plays_from_the_start_of_its_file_at_its_time() {
        let added = AddedAudio {
            start: TimelineTime::from_ms(1000.0),
            end: TimelineTime::from_ms(1500.0),
            pcm: tone(440.0, 5.0),
            volume: 1.0,
            boost: false,
        };
        let mut mixer = Mixer::new(&project(3), vec![], vec![added]);
        assert_eq!(peak(&render(&mut mixer, 1.0)), 0.0);
        let during = render(&mut mixer, 0.5);
        assert!(
            during[2].abs() < 0.2,
            "it starts at the file's beginning, near a zero crossing"
        );
        assert!(peak(&during) > 0.3);
        assert_eq!(peak(&render(&mut mixer, 0.5)), 0.0);
    }

    #[test]
    fn seeking_moves_the_sound_with_it() {
        let (first, second) = (tone(300.0, 1.0), tone(900.0, 1.0));
        let samples: Vec<i16> = first
            .samples
            .iter()
            .chain(second.samples.iter())
            .copied()
            .collect();
        let mut mixer = Mixer::new(&project(2), vec![source(Pcm::new(samples))], vec![]);
        mixer.seek(TimelineTime::from_ms(1000.0));
        assert_eq!(mixer.position(), TimelineTime::from_ms(1000.0));
        let measured = frequency(&render(&mut mixer, 0.5));
        assert!((measured - 900.0).abs() < 8.0, "got {measured} Hz");
    }

    #[test]
    fn rendering_in_small_blocks_matches_rendering_in_one() {
        // cpal asks for small blocks, and export asks for frame-sized ones.
        let mut project = project(2);
        project
            .timeline
            .apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 1.5,
            })
            .unwrap();
        let sources = vec![source(tone(440.0, 2.0))];
        let mut whole = Mixer::new(&project, sources.clone(), vec![]);
        let mut pieces = Mixer::new(&project, sources, vec![]);
        let one = render(&mut whole, 1.0);
        let mut joined = Vec::new();
        while joined.len() < one.len() {
            let mut block = vec![0.0; 512 * CHANNELS];
            pieces.render(&mut block);
            joined.extend(block);
        }
        joined.truncate(one.len());
        assert_eq!(one, joined);
    }

    #[test]
    fn the_next_playable_position_skips_gaps() {
        let mut project = project(4);
        project
            .timeline
            .apply(&Edit::SplitClip {
                at: TimelineTime::from_ms(1000.0),
            })
            .unwrap();
        let second = project.timeline.clips[1].id.clone();
        project
            .timeline
            .apply(&Edit::MoveClip {
                id: second,
                to: TimelineTime::from_ms(2000.0),
            })
            .unwrap();
        let mixer = Mixer::new(&project, vec![], vec![]);
        let rate = i64::from(SAMPLE_RATE);
        assert_eq!(mixer.next_playable(rate / 2), Some(rate / 2));
        assert_eq!(mixer.next_playable(rate * 3 / 2), Some(rate * 2));
        assert_eq!(mixer.next_playable(rate * 100), None);
    }
}
