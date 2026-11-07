//! Running Whisper locally. Nothing leaves the machine (§12).

use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::segment::{Cue, segment, words_from_tokens};
use crate::silence::{find_silences, to_whisper_input};

const WHISPER_RATE: u32 = 16_000;

/// Turns speech into captions.
///
/// `stereo_48k` is the audio, `language` is `auto` or a code like `en`, and `progress` is told how far
/// along it is, from 0 to 100.
///
/// # Errors
/// Returns why it couldn't be done, in words for the user.
pub fn transcribe(
    model: &Path,
    stereo_48k: &[i16],
    language: &str,
    progress: impl FnMut(i32) + 'static,
) -> Result<Vec<Cue>, String> {
    let audio = to_whisper_input(stereo_48k);
    if audio.is_empty() {
        return Err("There's no sound to transcribe.".to_string());
    }
    let silences = find_silences(&audio, WHISPER_RATE);

    let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|error| format!("The model couldn't be loaded: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("Transcription couldn't start: {error}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(if language == "auto" {
        Some("auto")
    } else {
        Some(language)
    });
    params.set_token_timestamps(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_progress_callback_safe(progress);

    state
        .full(params, &audio)
        .map_err(|error| format!("Transcription failed: {error}"))?;

    let mut tokens = Vec::new();
    for segment in state.as_iter() {
        for index in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(index) else {
                continue;
            };
            let Ok(text) = token.to_str_lossy() else {
                continue;
            };
            let data = token.token_data();
            // Whisper counts in hundredths of a second.
            tokens.push((
                text.into_owned(),
                data.t0 as f64 * 10.0,
                data.t1 as f64 * 10.0,
            ));
        }
    }
    Ok(segment(&words_from_tokens(&tokens), &silences))
}
