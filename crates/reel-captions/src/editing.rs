//! Changing captions by hand (§12): splitting at a word, merging with the next, and placing a
//! transcript's cues on the timeline.

use reel_core::time::{SourceTime, TimelineTime};
use reel_core::timeline::{Caption, CaptionWord, ItemId, MIN_ITEM, Timeline};

use crate::segment::Cue;

/// Puts a transcript's cues on the timeline. Their times are on the recording, so they go through the
/// clips, and anything cut out of the timeline is left out.
#[must_use]
pub fn captions_from_cues(timeline: &Timeline, cues: &[Cue]) -> Vec<Caption> {
    let place = |ms: f64| timeline.to_timeline(SourceTime::from_ms(ms));
    let mut result: Vec<Caption> = Vec::new();
    for cue in cues {
        let words: Vec<CaptionWord> = cue
            .words
            .iter()
            .filter_map(|word| {
                Some(CaptionWord {
                    text: word.text.clone(),
                    start: place(word.start_ms)?,
                    end: place(word.end_ms).or_else(|| place(word.start_ms))?,
                    leading_space: true,
                })
            })
            .collect();
        let start = place(cue.start_ms).or_else(|| words.first().map(|word| word.start));
        let end = place(cue.end_ms).or_else(|| words.last().map(|word| word.end));
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        if end - start < MIN_ITEM {
            continue;
        }
        let text = words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }
        result.push(Caption {
            id: format!("caption-{}", result.len() + 1).as_str().into(),
            start,
            end,
            text,
            words,
        });
    }
    result.sort_by_key(|caption| caption.start);
    // Moving through clips can squash neighbours together, and captions can't overlap.
    for index in 1..result.len() {
        let limit = result[index].start;
        if result[index - 1].end > limit {
            result[index - 1].end = limit;
        }
    }
    result.retain(|caption| caption.end - caption.start >= MIN_ITEM);
    result
}

/// Where to cut a caption's text. With word timing, it's the word boundary nearest the playhead.
/// Without it, the text is cut in proportion to the time, at a space.
fn split_text(
    caption: &Caption,
    at: TimelineTime,
) -> (String, String, Vec<CaptionWord>, Vec<CaptionWord>) {
    if caption.words.len() >= 2 {
        let boundary = (1..caption.words.len())
            .min_by_key(|index| (caption.words[*index].start - at).0.abs())
            .unwrap_or(1);
        let (first, second) = caption.words.split_at(boundary);
        let join = |words: &[CaptionWord]| {
            words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        return (join(first), join(second), first.to_vec(), second.to_vec());
    }
    let fraction = (at - caption.start).0 as f64 / (caption.end - caption.start).0.max(1) as f64;
    let chars: Vec<char> = caption.text.chars().collect();
    let target = (chars.len() as f64 * fraction) as usize;
    let spaces = chars
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_whitespace())
        .map(|(index, _)| index);
    let cut = spaces
        .min_by_key(|index| index.abs_diff(target))
        .unwrap_or_else(|| target.min(chars.len()));
    let first: String = chars[..cut].iter().collect();
    let second: String = chars[cut..].iter().collect();
    (
        first.trim().to_string(),
        second.trim().to_string(),
        Vec::new(),
        Vec::new(),
    )
}

/// Splits a caption at a time. Returns the new list, or `None` if there's no room there.
#[must_use]
pub fn split_caption(captions: &[Caption], id: &ItemId, at: TimelineTime) -> Option<Vec<Caption>> {
    let index = captions.iter().position(|caption| caption.id == *id)?;
    let caption = &captions[index];
    if at - caption.start < MIN_ITEM || caption.end - at < MIN_ITEM {
        return None;
    }
    let (first_text, second_text, first_words, second_words) = split_text(caption, at);
    let mut result = captions.to_vec();
    let first = Caption {
        end: at,
        text: first_text,
        words: first_words,
        ..caption.clone()
    };
    let second = Caption {
        id: format!("{}-b", caption.id.0).as_str().into(),
        start: at,
        text: second_text,
        words: second_words,
        ..caption.clone()
    };
    result[index] = first;
    result.insert(index + 1, second);
    Some(result)
}

/// Joins a caption with the one after it. Returns the new list, or `None` if there's no next one.
#[must_use]
pub fn merge_caption(captions: &[Caption], id: &ItemId) -> Option<Vec<Caption>> {
    let index = captions.iter().position(|caption| caption.id == *id)?;
    let next = captions.get(index + 1)?;
    let current = &captions[index];
    let mut words = current.words.clone();
    words.extend(next.words.iter().cloned());
    let merged = Caption {
        end: next.end,
        text: format!("{} {}", current.text, next.text).trim().to_string(),
        words,
        ..current.clone()
    };
    let mut result = captions.to_vec();
    result[index] = merged;
    result.remove(index + 1);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::Word;
    use reel_core::edit::{Edge, Edit};
    use reel_core::time::Micros;

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    fn cue(start: f64, end: f64, text: &str) -> Cue {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let step = (end - start) / parts.len() as f64;
        Cue {
            start_ms: start,
            end_ms: end,
            words: parts
                .iter()
                .enumerate()
                .map(|(index, word)| Word {
                    start_ms: start + index as f64 * step,
                    end_ms: start + (index + 1) as f64 * step,
                    text: (*word).to_string(),
                })
                .collect(),
        }
    }

    fn timeline() -> Timeline {
        Timeline::for_recording(Micros::from_secs(10))
    }

    fn caption(id: &str, start: f64, end: f64, text: &str) -> Caption {
        Caption {
            id: id.into(),
            start: at(start),
            end: at(end),
            text: text.to_string(),
            words: vec![],
        }
    }

    #[test]
    fn cues_land_at_their_times_with_a_full_length_clip() {
        let captions = captions_from_cues(&timeline(), &[cue(1000.0, 2000.0, "one two")]);
        assert_eq!(captions.len(), 1);
        assert_eq!(
            (captions[0].start, captions[0].end),
            (at(1000.0), at(2000.0))
        );
        assert_eq!(captions[0].text, "one two");
        assert_eq!(captions[0].words.len(), 2);
    }

    #[test]
    fn cues_follow_the_clips_when_the_start_is_trimmed() {
        let mut timeline = timeline();
        let id = timeline.clips[0].id.clone();
        timeline
            .apply(&Edit::TrimClip {
                id,
                edge: Edge::Start,
                to: at(4000.0),
            })
            .unwrap();
        // Footage from 5 s plays at 5 s on the timeline: a trim leaves things where they are.
        let captions = captions_from_cues(
            &timeline,
            &[cue(5000.0, 6000.0, "later"), cue(1000.0, 2000.0, "gone")],
        );
        assert_eq!(captions.len(), 1, "what's trimmed away is left out");
        assert_eq!(captions[0].text, "later");
    }

    #[test]
    fn cues_speed_up_with_their_clip() {
        let mut timeline = timeline();
        let id = timeline.clips[0].id.clone();
        timeline
            .apply(&Edit::SetClipSpeed { id, speed: 2.0 })
            .unwrap();
        let captions = captions_from_cues(&timeline, &[cue(2000.0, 4000.0, "fast")]);
        assert_eq!(
            (captions[0].start, captions[0].end),
            (at(1000.0), at(2000.0))
        );
    }

    #[test]
    fn captions_never_overlap_after_being_placed() {
        let captions = captions_from_cues(
            &timeline(),
            &[cue(1000.0, 2500.0, "a b"), cue(2000.0, 3000.0, "c d")],
        );
        for pair in captions.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
    }

    #[test]
    fn a_split_with_word_timing_lands_on_the_nearest_word() {
        let placed = captions_from_cues(&timeline(), &[cue(0.0, 4000.0, "one two three four")]);
        let id = placed[0].id.clone();
        let split = split_caption(&placed, &id, at(2100.0)).unwrap();
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].text, "one two");
        assert_eq!(split[1].text, "three four");
        assert_eq!(split[0].end, at(2100.0));
        assert_eq!(split[1].start, at(2100.0));
        assert_ne!(split[0].id, split[1].id);
    }

    #[test]
    fn a_split_without_word_timing_cuts_at_a_space() {
        let list = [caption("a", 0.0, 4000.0, "hello there world")];
        // 60% of the way through, and the nearest space to that is after "there".
        let split = split_caption(&list, &"a".into(), at(2400.0)).unwrap();
        assert_eq!(split[0].text, "hello there");
        assert_eq!(split[1].text, "world");
    }

    #[test]
    fn a_split_too_near_an_end_is_refused() {
        let list = [caption("a", 0.0, 4000.0, "hello there")];
        assert!(split_caption(&list, &"a".into(), at(50.0)).is_none());
        assert!(split_caption(&list, &"a".into(), at(3950.0)).is_none());
        assert!(split_caption(&list, &"missing".into(), at(2000.0)).is_none());
    }

    #[test]
    fn merging_joins_text_and_times() {
        let list = [
            caption("a", 0.0, 1000.0, "one"),
            caption("b", 2000.0, 3000.0, "two"),
            caption("c", 4000.0, 5000.0, "three"),
        ];
        let merged = merge_caption(&list, &"a".into()).unwrap();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].text, "one two");
        assert_eq!((merged[0].start, merged[0].end), (at(0.0), at(3000.0)));
        assert_eq!(merged[1].text, "three");
    }

    #[test]
    fn the_last_caption_has_nothing_to_merge_with() {
        let list = [caption("a", 0.0, 1000.0, "one")];
        assert!(merge_caption(&list, &"a".into()).is_none());
    }
}
