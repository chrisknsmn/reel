//! Finding the quiet parts of a recording. Recordly asks FFmpeg to do this, with
//! `silencedetect=noise=-30dB:d=0.5` (§36). These are the same settings.

use crate::segment::Silence;

const THRESHOLD_DB: f32 = -30.0;
const SHORTEST_MS: f64 = 500.0;
const FRAME_MS: f64 = 10.0;

/// The stretches of at least half a second below -30 dB, in mono samples at a given rate.
#[must_use]
pub fn find_silences(samples: &[f32], rate: u32) -> Vec<Silence> {
    let frame = ((f64::from(rate) * FRAME_MS / 1000.0) as usize).max(1);
    let threshold = 10.0_f32.powf(THRESHOLD_DB / 20.0);
    let mut found = Vec::new();
    let mut start: Option<usize> = None;
    let chunks = samples.chunks(frame);
    let total = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        let power: f32 =
            chunk.iter().map(|sample| sample * sample).sum::<f32>() / chunk.len() as f32;
        let quiet = power.sqrt() < threshold;
        match (quiet, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                push(&mut found, from, index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        push(&mut found, from, total);
    }
    found
}

fn push(found: &mut Vec<Silence>, from: usize, to: usize) {
    let (start_ms, end_ms) = (from as f64 * FRAME_MS, to as f64 * FRAME_MS);
    if end_ms - start_ms >= SHORTEST_MS {
        found.push(Silence { start_ms, end_ms });
    }
}

/// Turns interleaved 48 kHz stereo into what Whisper wants: 16 kHz mono. It averages each three frames,
/// which is a crude low-pass filter, but plenty for speech.
#[must_use]
pub fn to_whisper_input(stereo_48k: &[i16]) -> Vec<f32> {
    stereo_48k
        .chunks(6)
        .map(|chunk| {
            let sum: i32 = chunk.iter().map(|sample| i32::from(*sample)).sum();
            sum as f32 / chunk.len().max(1) as f32 / 32768.0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    fn tone(ms: u32) -> Vec<f32> {
        (0..RATE * ms / 1000)
            .map(|index| (index as f32 * 0.2).sin() * 0.5)
            .collect()
    }

    fn quiet(ms: u32) -> Vec<f32> {
        vec![0.0; (RATE * ms / 1000) as usize]
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 25.0
    }

    #[test]
    fn a_quiet_stretch_is_found_where_it_is() {
        let mut samples = tone(1000);
        samples.extend(quiet(800));
        samples.extend(tone(1000));
        let found = find_silences(&samples, RATE);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            near(found[0].start_ms, 1000.0) && near(found[0].end_ms, 1800.0),
            "{found:?}"
        );
    }

    #[test]
    fn short_quiet_stretches_dont_count() {
        let mut samples = tone(1000);
        samples.extend(quiet(300));
        samples.extend(tone(1000));
        assert!(find_silences(&samples, RATE).is_empty());
    }

    #[test]
    fn quiet_at_the_end_counts() {
        let mut samples = tone(500);
        samples.extend(quiet(1000));
        let found = find_silences(&samples, RATE);
        assert_eq!(found.len(), 1);
        assert!(near(found[0].end_ms, 1500.0));
    }

    #[test]
    fn faint_sound_counts_as_quiet_and_louder_doesnt() {
        // -40 dB is below the threshold, and -20 dB is above it.
        let level = |db: f32| 10.0_f32.powf(db / 20.0);
        let wave = |amplitude: f32| -> Vec<f32> {
            (0..RATE)
                .map(|index| (index as f32 * 0.2).sin() * amplitude * std::f32::consts::SQRT_2)
                .collect()
        };
        assert_eq!(find_silences(&wave(level(-40.0)), RATE).len(), 1);
        assert!(find_silences(&wave(level(-20.0)), RATE).is_empty());
    }

    #[test]
    fn stereo_becomes_a_third_as_many_mono_samples() {
        let stereo = vec![16384_i16; 48_000 * 2];
        let mono = to_whisper_input(&stereo);
        assert_eq!(mono.len(), 16_000);
        assert!((mono[0] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn the_channels_are_averaged() {
        let stereo = [32767, -32768, 32767, -32768, 32767, -32768];
        assert!(to_whisper_input(&stereo)[0].abs() < 0.01);
    }
}
