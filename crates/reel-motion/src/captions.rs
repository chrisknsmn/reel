//! How captions are laid out and animated. Measuring text is passed in, which keeps all of this
//! testable without any fonts.

use reel_core::project::{CaptionAnimation, CaptionStyle};
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::Caption;

/// How long a page takes to fade in and out.
const ANIMATION: Micros = Micros(180_000);
/// A gap shorter than this between pages doesn't fade out and back in.
const BRIDGE: Micros = Micros(500_000);

/// Sizes in pixels for a stage of a given width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionMetrics {
    pub font_px: f64,
    pub padding: (f64, f64),
    pub radius: f64,
    pub max_width: f64,
}

/// `font = max(14, size × (W × maxW%) / (1920 × 0.62))`.
#[must_use]
pub fn metrics(style: &CaptionStyle, stage_width: f64) -> CaptionMetrics {
    let max_width = stage_width * style.max_width / 100.0;
    let font = (style.font_size * max_width / (1920.0 * 0.62)).max(14.0);
    CaptionMetrics {
        font_px: font,
        padding: (1.1 * font, 0.78 * font),
        radius: style.box_radius * font / 30.0,
        max_width,
    }
}

/// Wraps words into lines that fit, balanced so no line is left short. This is dynamic programming
/// over every possible set of breaks. A word too wide for any line gets one
/// to itself.
pub fn wrap_words(
    words: &[String],
    max_width: f64,
    measure: &mut dyn FnMut(&str) -> f64,
) -> Vec<String> {
    if words.is_empty() {
        return Vec::new();
    }
    let n = words.len();
    let space = measure(" ");
    let widths: Vec<f64> = words.iter().map(|word| measure(word)).collect();
    let line_width = |from: usize, to: usize| -> f64 {
        widths[from..to].iter().sum::<f64>() + space * (to - from - 1) as f64
    };

    // best[i] is the lowest cost for the words from i to the end.
    let mut best = vec![f64::INFINITY; n + 1];
    let mut next_break = vec![n; n + 1];
    best[n] = 0.0;
    for from in (0..n).rev() {
        for to in (from + 1)..=n {
            let width = line_width(from, to);
            if width > max_width && to > from + 1 {
                break;
            }
            let slack = (max_width - width).max(0.0);
            let cost = if to == n {
                slack * slack * 0.18
            } else {
                let fullness = width / max_width;
                let short = if fullness < 0.72 {
                    (0.72 - fullness) * 26000.0
                } else {
                    0.0
                };
                slack * slack + short
            };
            let total = cost + best[to];
            if total < best[from] {
                best[from] = total;
                next_break[from] = to;
            }
        }
    }

    let mut lines = Vec::new();
    let mut from = 0;
    while from < n {
        let to = next_break[from];
        lines.push(words[from..to].join(" "));
        from = to;
    }
    lines
}

/// One screenful of a caption.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub start: TimelineTime,
    pub end: TimelineTime,
    pub lines: Vec<String>,
    /// Whether the next page follows straight on, in which case this doesn't fade out.
    pub bridged: bool,
    /// Whether the previous page led straight in, in which case this doesn't fade in.
    pub continued: bool,
}

/// Splits captions into pages of up to `max_rows` lines, each getting a share of its caption's time.
/// A manual line break in a caption always stays a line break.
pub fn pages(
    captions: &[Caption],
    style: &CaptionStyle,
    max_width: f64,
    measure: &mut dyn FnMut(&str) -> f64,
) -> Vec<Page> {
    let rows = style.max_rows.max(1) as usize;
    let mut result: Vec<Page> = Vec::new();
    for caption in captions {
        let mut lines: Vec<String> = Vec::new();
        for paragraph in caption.text.split('\n') {
            let words: Vec<String> = paragraph.split_whitespace().map(str::to_string).collect();
            lines.extend(wrap_words(&words, max_width, measure));
        }
        if lines.is_empty() {
            continue;
        }
        let chunks: Vec<&[String]> = lines.chunks(rows).collect();
        let total_chars: usize = chunks
            .iter()
            .map(|chunk| chunk.iter().map(String::len).sum::<usize>())
            .sum();
        let duration = caption.end - caption.start;
        let mut cursor = caption.start;
        for (index, chunk) in chunks.iter().enumerate() {
            let chars: usize = chunk.iter().map(String::len).sum();
            let end = if index + 1 == chunks.len() {
                caption.end
            } else {
                cursor + duration.scaled(chars as f64 / total_chars.max(1) as f64)
            };
            result.push(Page {
                start: cursor,
                end,
                lines: chunk.to_vec(),
                bridged: false,
                continued: false,
            });
            cursor = end;
        }
    }
    for index in 1..result.len() {
        let gap = result[index].start - result[index - 1].end;
        if gap < BRIDGE {
            result[index - 1].bridged = true;
            // Bridging holds the page on screen until the next one takes over.
            result[index - 1].end = result[index].start;
            result[index].continued = true;
        }
    }
    result
}

/// How a page looks at a moment in its animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageMotion {
    pub opacity: f64,
    /// Pixels at a 1080p stage, positive downward.
    pub translate_y: f64,
    pub scale: f64,
}

/// `v = min(enter, exit)` over 180 ms, shaped by the animation style.
#[must_use]
pub fn page_motion(page: &Page, time: TimelineTime, animation: CaptionAnimation) -> PageMotion {
    let window = ANIMATION.0 as f64;
    let enter = if page.continued {
        1.0
    } else {
        ((time - page.start).0 as f64 / window).clamp(0.0, 1.0)
    };
    let exit = if page.bridged {
        1.0
    } else {
        ((page.end - time).0 as f64 / window).clamp(0.0, 1.0)
    };
    let v = enter.min(exit);
    match animation {
        CaptionAnimation::None => PageMotion {
            opacity: 1.0,
            translate_y: 0.0,
            scale: 1.0,
        },
        CaptionAnimation::Fade => PageMotion {
            opacity: 0.3 + 0.7 * v,
            translate_y: 0.0,
            scale: 1.0,
        },
        CaptionAnimation::Rise => PageMotion {
            opacity: 0.25 + 0.75 * v,
            translate_y: 18.0 * (1.0 - v),
            scale: 0.985 + 0.015 * v,
        },
        CaptionAnimation::Pop => PageMotion {
            opacity: 0.35 + 0.65 * v,
            translate_y: 8.0 * (1.0 - v),
            scale: 0.94 + 0.06 * v,
        },
    }
}

/// The page showing at a time.
#[must_use]
pub fn page_at(pages: &[Page], time: TimelineTime) -> Option<&Page> {
    pages
        .iter()
        .find(|page| time >= page.start && time < page.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10 pixels wide, which keeps the sums easy.
    fn measure(text: &str) -> f64 {
        text.chars().count() as f64 * 10.0
    }

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    fn cue(start: f64, end: f64, text: &str) -> Caption {
        Caption {
            id: "c".into(),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            text: text.to_string(),
            words: vec![],
        }
    }

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    #[test]
    fn sizes_follow_the_documented_formula() {
        let style = CaptionStyle::default();
        let found = metrics(&style, 1920.0);
        assert!(
            (found.font_px - 30.0).abs() < 1e-9,
            "at the defaults, the font is its set size"
        );
        assert!((found.padding.0 - 33.0).abs() < 1e-9);
        assert!((found.padding.1 - 23.4).abs() < 1e-9);
        assert!((found.radius - 17.5).abs() < 1e-9);
        assert!((found.max_width - 1190.4).abs() < 1e-9);
        assert!(
            (metrics(&style, 3840.0).font_px - 60.0).abs() < 1e-9,
            "it scales with the stage"
        );
        assert_eq!(
            metrics(&style, 100.0).font_px,
            14.0,
            "but never gets too small to read"
        );
    }

    #[test]
    fn short_text_stays_on_one_line() {
        assert_eq!(
            wrap_words(&words("hello there"), 500.0, &mut measure),
            ["hello there"]
        );
    }

    #[test]
    fn no_line_is_wider_than_allowed() {
        let lines = wrap_words(
            &words("the quick brown fox jumps over the lazy dog again and again"),
            200.0,
            &mut measure,
        );
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(measure(line) <= 200.0, "{line} is too wide");
        }
    }

    #[test]
    fn all_the_words_survive_in_order() {
        let text = "the quick brown fox jumps over the lazy dog";
        let lines = wrap_words(&words(text), 150.0, &mut measure);
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn lines_look_ahead_rather_than_filling_greedily() {
        // Filling greedily gives "aaa bbbbbbb cc" and then "dddd" on its own, which costs a little
        // more than breaking one word sooner.
        let lines = wrap_words(&words("aaa bbbbbbb cc dddd"), 140.0, &mut measure);
        assert_eq!(lines, ["aaa bbbbbbb", "cc dddd"]);
    }

    #[test]
    fn a_short_last_line_is_fine_but_a_short_early_one_isnt() {
        // The costs forgive the last line, so a full line and then a short one wins here.
        let lines = wrap_words(&words("aaaa bbbb cccc dddd"), 140.0, &mut measure);
        assert_eq!(lines, ["aaaa bbbb cccc", "dddd"]);
    }

    #[test]
    fn a_word_too_long_for_any_line_gets_its_own() {
        let lines = wrap_words(&words("hi supercalifragilistic hi"), 100.0, &mut measure);
        assert_eq!(lines, ["hi", "supercalifragilistic", "hi"]);
    }

    #[test]
    fn no_words_means_no_lines() {
        assert!(wrap_words(&[], 100.0, &mut measure).is_empty());
    }

    #[test]
    fn pages_hold_up_to_the_set_number_of_rows() {
        let style = CaptionStyle {
            max_rows: 2,
            ..CaptionStyle::default()
        };
        let found = pages(
            &[cue(0.0, 3000.0, "one two three four five six")],
            &style,
            40.0,
            &mut measure,
        );
        assert!(found.iter().all(|page| page.lines.len() <= 2));
        assert!(found.len() >= 2);
    }

    #[test]
    fn pages_share_out_the_captions_time_by_length() {
        let style = CaptionStyle::default();
        let found = pages(
            &[cue(1000.0, 3000.0, "aaaa aaaa")],
            &style,
            50.0,
            &mut measure,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].start, at(1000.0));
        assert_eq!(found[0].end, at(2000.0));
        assert_eq!(found[1].end, at(3000.0));
    }

    #[test]
    fn a_manual_line_break_is_kept() {
        let style = CaptionStyle {
            max_rows: 4,
            ..CaptionStyle::default()
        };
        let found = pages(
            &[cue(0.0, 1000.0, "first\nsecond")],
            &style,
            500.0,
            &mut measure,
        );
        assert_eq!(found[0].lines, ["first", "second"]);
    }

    #[test]
    fn close_captions_bridge_and_distant_ones_dont() {
        let style = CaptionStyle::default();
        let found = pages(
            &[
                cue(0.0, 1000.0, "a"),
                cue(1400.0, 2000.0, "b"),
                cue(5000.0, 6000.0, "c"),
            ],
            &style,
            500.0,
            &mut measure,
        );
        assert!(found[0].bridged);
        assert_eq!(
            found[0].end,
            at(1400.0),
            "the first holds until the second takes over"
        );
        assert!(found[1].continued);
        assert!(!found[1].bridged);
        assert!(!found[2].continued);
    }

    #[test]
    fn empty_captions_make_no_pages() {
        assert!(
            pages(
                &[cue(0.0, 1000.0, "   ")],
                &CaptionStyle::default(),
                500.0,
                &mut measure
            )
            .is_empty()
        );
    }

    #[test]
    fn the_right_page_shows_at_the_right_time() {
        let style = CaptionStyle::default();
        let found = pages(
            &[cue(1000.0, 2000.0, "a"), cue(5000.0, 6000.0, "b")],
            &style,
            500.0,
            &mut measure,
        );
        assert_eq!(page_at(&found, at(500.0)), None);
        assert_eq!(page_at(&found, at(1500.0)).unwrap().lines, ["a"]);
        assert_eq!(page_at(&found, at(3000.0)), None);
        assert_eq!(page_at(&found, at(5500.0)).unwrap().lines, ["b"]);
    }

    fn lone_page() -> Page {
        Page {
            start: at(1000.0),
            end: at(2000.0),
            lines: vec!["a".to_string()],
            bridged: false,
            continued: false,
        }
    }

    #[test]
    fn fade_runs_for_180_ms_each_way() {
        let page = lone_page();
        let opacity = |ms| page_motion(&page, at(ms), CaptionAnimation::Fade).opacity;
        assert!((opacity(1000.0) - 0.3).abs() < 1e-9);
        assert!((opacity(1090.0) - 0.65).abs() < 1e-9);
        assert_eq!(opacity(1500.0), 1.0);
        assert!((opacity(1910.0) - 0.65).abs() < 1e-9);
    }

    #[test]
    fn rise_and_pop_move_and_scale_as_documented() {
        let page = lone_page();
        let rise = page_motion(&page, at(1000.0), CaptionAnimation::Rise);
        assert_eq!((rise.translate_y, rise.scale), (18.0, 0.985));
        let pop = page_motion(&page, at(1000.0), CaptionAnimation::Pop);
        assert_eq!((pop.translate_y, pop.scale), (8.0, 0.94));
        let settled = page_motion(&page, at(1500.0), CaptionAnimation::Pop);
        assert_eq!(
            (settled.opacity, settled.translate_y, settled.scale),
            (1.0, 0.0, 1.0)
        );
    }

    #[test]
    fn a_bridged_page_doesnt_fade_where_it_joins() {
        let page = Page {
            bridged: true,
            continued: true,
            ..lone_page()
        };
        assert_eq!(
            page_motion(&page, at(1000.0), CaptionAnimation::Fade).opacity,
            1.0
        );
        assert_eq!(
            page_motion(&page, at(1999.0), CaptionAnimation::Fade).opacity,
            1.0
        );
    }

    #[test]
    fn no_animation_is_steady() {
        let motion = page_motion(&lone_page(), at(1000.0), CaptionAnimation::None);
        assert_eq!(
            (motion.opacity, motion.translate_y, motion.scale),
            (1.0, 0.0, 1.0)
        );
    }
}
