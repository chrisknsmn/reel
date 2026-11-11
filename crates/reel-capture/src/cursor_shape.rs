//! Telling which cursor is showing, by comparing its shape with the standard ones. A match above
//! the thresholds is too weak to trust, and it's called an arrow.

use reel_motion::cursor::CursorShape;

pub const SAMPLE_SIZE: usize = 32;
const THRESHOLD: i64 = 12_000;

/// A cursor's shape, reduced to something quick to compare.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub aspect: f64,
    pub hotspot_x: f64,
    pub hotspot_y: f64,
    /// One value per pixel of a 32×32 thumbnail: 255 where it's solid, 0 where it isn't.
    pub samples: Vec<u8>,
}

/// How different two cursors are. Lower is more alike.
#[must_use]
pub fn score(a: &Signature, b: &Signature) -> i64 {
    let image: i64 = a
        .samples
        .iter()
        .zip(&b.samples)
        .map(|(x, y)| (i64::from(*x) - i64::from(*y)).abs())
        .sum();
    let aspect = ((a.aspect - b.aspect).abs() * 1800.0) as i64;
    let hotspot =
        (((a.hotspot_x - b.hotspot_x).abs() + (a.hotspot_y - b.hotspot_y).abs()) * 2200.0) as i64;
    image + aspect + hotspot
}

fn threshold_for(shape: CursorShape) -> i64 {
    match shape {
        CursorShape::Text => 28_000,
        CursorShape::Crosshair => 32_000,
        CursorShape::OpenHand | CursorShape::ClosedHand => 3_200,
        _ => THRESHOLD,
    }
}

/// The closest known cursor, if it's close enough to trust.
#[must_use]
pub fn classify(current: &Signature, known: &[(CursorShape, Signature)]) -> CursorShape {
    let best = known
        .iter()
        .map(|(shape, signature)| (*shape, score(current, signature)))
        .min_by_key(|(_, score)| *score);
    match best {
        Some((shape, score)) if score <= threshold_for(shape) => shape,
        _ => CursorShape::Arrow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(from: usize, to: usize) -> Signature {
        let mut samples = vec![0; SAMPLE_SIZE * SAMPLE_SIZE];
        for y in from..to {
            for x in from..to {
                samples[y * SAMPLE_SIZE + x] = 255;
            }
        }
        Signature {
            aspect: 1.0,
            hotspot_x: 0.5,
            hotspot_y: 0.5,
            samples,
        }
    }

    #[test]
    fn identical_cursors_score_nothing() {
        assert_eq!(score(&square(4, 20), &square(4, 20)), 0);
    }

    #[test]
    fn each_part_of_the_signature_counts() {
        let base = square(4, 20);
        assert_eq!(
            score(
                &base,
                &Signature {
                    aspect: 2.0,
                    ..base.clone()
                }
            ),
            1800
        );
        assert_eq!(
            score(
                &base,
                &Signature {
                    hotspot_x: 1.0,
                    ..base.clone()
                }
            ),
            1100
        );
        assert_eq!(
            score(&square(4, 5), &square(5, 6)),
            510,
            "one pixel off in each is two differences"
        );
    }

    #[test]
    fn the_closest_known_cursor_wins() {
        let known = [
            (CursorShape::Pointer, square(4, 12)),
            (CursorShape::NotAllowed, square(16, 30)),
        ];
        assert_eq!(classify(&square(4, 12), &known), CursorShape::Pointer);
        assert_eq!(classify(&square(16, 30), &known), CursorShape::NotAllowed);
    }

    #[test]
    fn nothing_close_counts_as_an_arrow() {
        let known = [(CursorShape::Pointer, square(0, 4))];
        assert_eq!(classify(&square(10, 30), &known), CursorShape::Arrow);
        assert_eq!(classify(&square(10, 30), &[]), CursorShape::Arrow);
    }

    #[test]
    fn hands_need_a_much_closer_match_than_text() {
        let close = square(4, 12);
        // About 20 pixels different: 5,100, which is too far for a hand but fine for text.
        let mut near = close.clone();
        for sample in near.samples.iter_mut().rev().take(20) {
            *sample = 255;
        }
        assert_eq!(
            classify(&near, &[(CursorShape::OpenHand, close.clone())]),
            CursorShape::Arrow
        );
        assert_eq!(
            classify(&near, &[(CursorShape::Text, close)]),
            CursorShape::Text
        );
    }
}
