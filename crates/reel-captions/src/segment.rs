//! Turning a transcript's words into captions (§36).

/// A word with its timing, in milliseconds on the recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

/// A stretch with nothing to hear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Silence {
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One caption's worth of words.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start_ms: f64,
    pub end_ms: f64,
    pub words: Vec<Word>,
}

impl Cue {
    /// The cue's words as a line of text.
    #[must_use]
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

const DROP_SILENCE_MS: f64 = 1500.0;
const BREAK_GAP_MS: f64 = 700.0;
const LONGEST_PHRASE_MS: f64 = 12_000.0;
const MERGE_SHORTER_THAN_MS: f64 = 800.0;
const MERGE_GAP_MS: f64 = 400.0;
const MERGE_TOTAL_MS: f64 = 2500.0;
const MERGE_TOTAL_CHARS: usize = 80;
const PAD_MS: f64 = 80.0;
/// How close to a silence's end a run of words has to reach to count as smeared.
const SMEAR_REACH_MS: f64 = 300.0;
/// How soon after a silence the next word has to start, for a run to flow into it.
const SMEAR_LEADS_ON_MS: f64 = 1000.0;
const ABBREVIATIONS: [&str; 10] = [
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc",
];

/// Whether a word ends a sentence, rather than an abbreviation or an initial.
#[must_use]
pub fn ends_sentence(word: &str) -> bool {
    let trimmed = word.trim_end_matches(['"', '\'', ')', ']', '”', '’']);
    let Some(last) = trimmed.chars().last() else {
        return false;
    };
    if !matches!(last, '.' | '?' | '!' | '…' | '。' | '！' | '？') {
        return false;
    }
    if last != '.' {
        return true;
    }
    let stem = trimmed.trim_end_matches('.').to_lowercase();
    if ABBREVIATIONS.contains(&stem.as_str()) {
        return false;
    }
    // A single letter is an initial, like the J. in J. Smith.
    !(stem.chars().count() == 1 && stem.chars().all(char::is_alphabetic))
}

fn silence_in_gap(silences: &[Silence], from: f64, to: f64) -> bool {
    silences.iter().any(|silence| {
        silence.end_ms - silence.start_ms >= DROP_SILENCE_MS
            && silence.start_ms < to
            && silence.end_ms > from
    })
}

fn inside_long_silence(word: &Word, silences: &[Silence]) -> bool {
    silences.iter().any(|silence| {
        silence.end_ms - silence.start_ms >= DROP_SILENCE_MS
            && word.start_ms >= silence.start_ms
            && word.end_ms <= silence.end_ms
    })
}

/// Whisper's timings smear across a pause: real words just after it get times inside it, packed up
/// against what came before. Dropping them as §36 says would lose real speech after every pause, which
/// a test with real audio showed. Those runs flow right up to the end of the silence and on into the next
/// word, unlike words Whisper made up, which sit stranded in the middle. So runs like that are moved
/// onto the speech they belong to, sharing its time by length.
fn retime_smeared(words: &[Word], silences: &[Silence]) -> Vec<Word> {
    let mut result = words.to_vec();
    for silence in silences
        .iter()
        .filter(|silence| silence.end_ms - silence.start_ms >= DROP_SILENCE_MS)
    {
        let inside: Vec<usize> = result
            .iter()
            .enumerate()
            .filter(|(_, word)| word.start_ms >= silence.start_ms && word.end_ms <= silence.end_ms)
            .map(|(index, _)| index)
            .collect();
        let (Some(first), Some(last)) = (inside.first().copied(), inside.last().copied()) else {
            continue;
        };
        let Some(next) = result.get(last + 1).cloned() else {
            continue;
        };
        let reaches_the_end = result[last].end_ms >= silence.end_ms - SMEAR_REACH_MS;
        let leads_on = next.start_ms - silence.end_ms <= SMEAR_LEADS_ON_MS;
        if !reaches_the_end || !leads_on {
            continue;
        }
        let span_start = silence.end_ms;
        let span = (next.end_ms - span_start).max(1.0);
        let lengths: Vec<f64> = result[first..=last + 1]
            .iter()
            .map(|word| word.text.chars().count().max(1) as f64)
            .collect();
        let total: f64 = lengths.iter().sum();
        let mut at = span_start;
        for (word, length) in result[first..=last + 1].iter_mut().zip(&lengths) {
            let share = span * length / total;
            word.start_ms = at;
            word.end_ms = at + share;
            at += share;
        }
    }
    result
}

/// Groups words into captions. Whisper makes up words during long silences, so those are dropped.
#[must_use]
pub fn segment(words: &[Word], silences: &[Silence]) -> Vec<Cue> {
    let words = retime_smeared(words, silences);
    let mut cues: Vec<Cue> = Vec::new();
    let mut current: Vec<Word> = Vec::new();

    let mut flush = |current: &mut Vec<Word>| {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            cues.push(Cue {
                start_ms: first.start_ms,
                end_ms: last.end_ms,
                words: std::mem::take(current),
            });
        }
    };

    for word in words
        .iter()
        .filter(|word| !inside_long_silence(word, silences))
    {
        if let Some(previous) = current.last() {
            let gap = word.start_ms - previous.end_ms;
            let phrase = word.end_ms - current[0].start_ms;
            if gap >= BREAK_GAP_MS
                || phrase >= LONGEST_PHRASE_MS
                || silence_in_gap(silences, previous.end_ms, word.start_ms)
            {
                flush(&mut current);
            }
        }
        current.push(word.clone());
        if ends_sentence(&word.text) {
            flush(&mut current);
        }
    }
    flush(&mut current);

    pad(merge_short(cues))
}

fn merge_short(cues: Vec<Cue>) -> Vec<Cue> {
    let mut merged: Vec<Cue> = Vec::with_capacity(cues.len());
    for cue in cues {
        if let Some(previous) = merged.last_mut() {
            let both_short = previous.end_ms - previous.start_ms < MERGE_SHORTER_THAN_MS
                && cue.end_ms - cue.start_ms < MERGE_SHORTER_THAN_MS;
            let gap = cue.start_ms - previous.end_ms;
            let total = cue.end_ms - previous.start_ms;
            let chars = previous.text().chars().count() + 1 + cue.text().chars().count();
            if both_short
                && gap <= MERGE_GAP_MS
                && total <= MERGE_TOTAL_MS
                && chars <= MERGE_TOTAL_CHARS
            {
                previous.end_ms = cue.end_ms;
                previous.words.extend(cue.words);
                continue;
            }
        }
        merged.push(cue);
    }
    merged
}

fn pad(mut cues: Vec<Cue>) -> Vec<Cue> {
    // Gaps are measured before anything moves, or each cue would see its neighbour's padding as part of
    // the gap and pad less than it should.
    let spans: Vec<(f64, f64)> = cues.iter().map(|cue| (cue.start_ms, cue.end_ms)).collect();
    let half_gap = |index: usize| ((spans[index + 1].0 - spans[index].1) / 2.0).clamp(0.0, PAD_MS);
    for (index, cue) in cues.iter_mut().enumerate() {
        let before = if index == 0 {
            PAD_MS
        } else {
            half_gap(index - 1)
        };
        let after = if index + 1 == spans.len() {
            PAD_MS
        } else {
            half_gap(index)
        };
        cue.start_ms = (cue.start_ms - before).max(0.0);
        cue.end_ms += after;
    }
    cues
}

/// Whisper's tokens can be parts of words. A token without a leading space joins the one before (§36).
#[must_use]
pub fn words_from_tokens(tokens: &[(String, f64, f64)]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    for (text, start, end) in tokens {
        // Special tokens look like `[_BEG_]` or `<|endoftext|>`, and aren't speech.
        if text.is_empty() || text.starts_with("[_") || text.starts_with("<|") {
            continue;
        }
        let joins = !text.starts_with(char::is_whitespace);
        match words.last_mut() {
            Some(word) if joins => {
                word.text.push_str(text);
                word.end_ms = *end;
            }
            _ => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    words.push(Word {
                        start_ms: *start,
                        end_ms: *end,
                        text: trimmed.to_string(),
                    });
                }
            }
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start: f64, end: f64, text: &str) -> Word {
        Word {
            start_ms: start,
            end_ms: end,
            text: text.to_string(),
        }
    }

    /// Words 300 ms long, one after another.
    fn run(from: f64, text: &str) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(index, text)| {
                let start = from + index as f64 * 300.0;
                word(start, start + 290.0, text)
            })
            .collect()
    }

    #[test]
    fn sentence_ends_are_recognised() {
        for yes in [
            "done.",
            "really?",
            "wow!",
            "so…",
            "終わり。",
            "何！",
            "何？",
            "end.\"",
            "(end.)",
        ] {
            assert!(ends_sentence(yes), "{yes}");
        }
        for no in ["word", "word,", ""] {
            assert!(!ends_sentence(no), "{no}");
        }
    }

    #[test]
    fn abbreviations_and_initials_dont_end_sentences() {
        for no in ["Mr.", "mrs.", "Dr.", "vs.", "etc.", "J."] {
            assert!(!ends_sentence(no), "{no}");
        }
        assert!(ends_sentence("Smith."));
    }

    #[test]
    fn a_sentence_end_starts_a_new_caption() {
        let cues = segment(&run(0.0, "one two three. four five six."), &[]);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text(), "one two three.");
        assert_eq!(cues[1].text(), "four five six.");
    }

    #[test]
    fn titles_stay_in_the_sentence() {
        let cues = segment(&run(0.0, "ask Dr. Smith about it."), &[]);
        assert_eq!(cues.len(), 1);
    }

    #[test]
    fn a_long_gap_starts_a_new_caption() {
        let mut words = run(0.0, "one two three four");
        words.extend(run(2000.0, "five six seven eight"));
        let cues = segment(&words, &[]);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1].words[0].text, "five");
    }

    #[test]
    fn a_short_gap_doesnt() {
        let mut words = run(0.0, "one two three");
        words.extend(run(1400.0, "four five six"));
        assert_eq!(segment(&words, &[]).len(), 1);
    }

    #[test]
    fn a_very_long_phrase_is_broken_up() {
        let text = "word ".repeat(100);
        let cues = segment(&run(0.0, &text), &[]);
        assert!(cues.len() >= 2);
        for cue in &cues {
            assert!(
                cue.end_ms - cue.start_ms <= LONGEST_PHRASE_MS + PAD_MS * 2.0 + 300.0,
                "{cue:?}"
            );
        }
    }

    #[test]
    fn words_inside_a_long_silence_are_dropped() {
        let words = vec![
            word(0.0, 300.0, "real."),
            word(2000.0, 2300.0, "ghost"),
            word(5000.0, 5300.0, "real."),
        ];
        let silences = [Silence {
            start_ms: 500.0,
            end_ms: 4500.0,
        }];
        let cues = segment(&words, &silences);
        let text: Vec<String> = cues.iter().map(Cue::text).collect();
        assert!(!text.join(" ").contains("ghost"), "{text:?}");
    }

    #[test]
    fn real_words_smeared_into_a_pause_are_kept_and_moved_after_it() {
        // These timings come from Whisper's tiny model, on real speech with a 2.5 s pause in it.
        let words = vec![
            word(2170.0, 2840.0, "captions."),
            word(2880.0, 3380.0, "And"),
            word(3380.0, 4170.0, "this"),
            word(4180.0, 4510.0, "is"),
            word(4540.0, 5090.0, "the"),
            word(5150.0, 5580.0, "second"),
            word(6260.0, 7120.0, "part."),
        ];
        let silences = [Silence {
            start_ms: 2720.0,
            end_ms: 5630.0,
        }];
        let cues = segment(&words, &silences);
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues[1].text(), "And this is the second part.");
        assert!(
            cues[1].start_ms >= 5500.0,
            "it starts after the pause: {cues:?}"
        );
        assert!(
            (cues[1].end_ms - 7200.0).abs() < 1.0,
            "and still ends with the speech: {cues:?}"
        );
        for pair in cues[1].words.windows(2) {
            assert!(
                pair[0].end_ms <= pair[1].start_ms,
                "the words stay in order: {pair:?}"
            );
        }
    }

    #[test]
    fn made_up_words_stranded_in_a_pause_are_still_dropped() {
        // In the middle, not touching the speech after it.
        let words = vec![
            word(0.0, 500.0, "real."),
            word(2000.0, 2500.0, "Thanks"),
            word(2500.0, 3000.0, "for"),
            word(3000.0, 3200.0, "watching."),
            word(5000.0, 5500.0, "real."),
        ];
        let silences = [Silence {
            start_ms: 600.0,
            end_ms: 4900.0,
        }];
        let text: Vec<String> = segment(&words, &silences).iter().map(Cue::text).collect();
        assert_eq!(text.join(" "), "real. real.");
    }

    #[test]
    fn words_at_the_end_of_a_pause_with_nothing_after_are_dropped() {
        let words = vec![word(0.0, 500.0, "real."), word(4000.0, 4800.0, "ghost")];
        let silences = [Silence {
            start_ms: 600.0,
            end_ms: 4900.0,
        }];
        let text: Vec<String> = segment(&words, &silences).iter().map(Cue::text).collect();
        assert_eq!(text.join(" "), "real.");
    }

    #[test]
    fn a_short_silence_doesnt_drop_anything() {
        let words = vec![word(600.0, 900.0, "kept.")];
        let silences = [Silence {
            start_ms: 500.0,
            end_ms: 1000.0,
        }];
        assert_eq!(segment(&words, &silences).len(), 1);
    }

    #[test]
    fn short_neighbours_merge() {
        let words = vec![word(0.0, 400.0, "Yes."), word(500.0, 900.0, "Okay.")];
        let cues = segment(&words, &[]);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text(), "Yes. Okay.");
    }

    #[test]
    fn neighbours_too_far_apart_or_too_long_dont() {
        let far = vec![word(0.0, 400.0, "Yes."), word(1000.0, 1400.0, "Okay.")];
        assert_eq!(segment(&far, &[]).len(), 2, "the gap is too big");
        let long = vec![word(0.0, 900.0, "Yes."), word(1000.0, 1400.0, "Okay.")];
        assert_eq!(segment(&long, &[]).len(), 2, "one isn't short");
    }

    #[test]
    fn padding_is_limited_to_half_the_gap() {
        let words = vec![
            word(1000.0, 2000.0, "first."),
            word(2040.0, 3000.0, "second."),
        ];
        let cues = segment(&words, &[]);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 920.0, "the full 80 ms before the first");
        assert_eq!(cues[0].end_ms, 2020.0, "half the 40 ms gap");
        assert_eq!(cues[1].start_ms, 2020.0);
        assert_eq!(cues[1].end_ms, 3080.0);
    }

    #[test]
    fn padding_never_goes_before_the_start() {
        let cues = segment(&[word(10.0, 500.0, "first.")], &[]);
        assert_eq!(cues[0].start_ms, 0.0);
    }

    #[test]
    fn captions_never_overlap() {
        let mut words = Vec::new();
        for index in 0..50 {
            let start = f64::from(index) * 350.0;
            let text = if index % 4 == 3 { "end." } else { "word" };
            words.push(word(start, start + 340.0, text));
        }
        let cues = segment(&words, &[]);
        for pair in cues.windows(2) {
            assert!(pair[0].end_ms <= pair[1].start_ms, "{pair:?}");
        }
    }

    #[test]
    fn nothing_in_means_nothing_out() {
        assert!(segment(&[], &[]).is_empty());
    }

    #[test]
    fn tokens_join_into_words() {
        let token = |text: &str, start: f64, end: f64| (text.to_string(), start, end);
        let words = words_from_tokens(&[
            token("[_BEG_]", 0.0, 0.0),
            token(" Hel", 0.0, 100.0),
            token("lo", 100.0, 200.0),
            token(" there", 250.0, 400.0),
            token(".", 400.0, 450.0),
            token("<|endoftext|>", 450.0, 450.0),
        ]);
        assert_eq!(
            words,
            [word(0.0, 200.0, "Hello"), word(250.0, 450.0, "there.")]
        );
    }
}
