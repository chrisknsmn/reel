//! CSS gradients, as wallpapers use them.

/// A colour in 0 to 1, not premultiplied.
pub type Rgba = [f32; 4];

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A direction in radians, clockwise from pointing up, as in CSS.
    Linear {
        angle: f32,
    },
    Radial,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub shape: Shape,
    /// Colour stops from 0 to 1, in order.
    pub stops: Vec<(f32, Rgba)>,
}

impl Gradient {
    /// Reads CSS text, angle included, so export draws a gradient the way the preview does.
    #[must_use]
    pub fn parse(css: &str) -> Option<Self> {
        let css = css.trim();
        let open = css.find('(')?;
        let close = css.rfind(')')?;
        let name = css[..open].trim().to_ascii_lowercase();
        let parts = split_arguments(&css[open + 1..close]);

        let (shape, first_stop) = if name.contains("radial") {
            // Any shape and position before the stops are ignored: the gradient is centred.
            let has_shape = parts
                .first()
                .is_some_and(|part| parse_color(first_word(part)).is_none());
            (Shape::Radial, usize::from(has_shape))
        } else if name.contains("linear") {
            match parts.first().and_then(|part| parse_direction(part)) {
                Some(angle) => (Shape::Linear { angle }, 1),
                // CSS's default is top to bottom.
                None => (
                    Shape::Linear {
                        angle: std::f32::consts::PI,
                    },
                    0,
                ),
            }
        } else {
            return None;
        };

        let raw: Vec<(Option<f32>, Rgba)> = parts[first_stop..]
            .iter()
            .filter_map(|part| {
                let (color, rest) = split_color(part)?;
                let position = rest
                    .trim()
                    .strip_suffix('%')
                    .and_then(|n| n.trim().parse::<f32>().ok())
                    .map(|n| n / 100.0);
                Some((position, color))
            })
            .collect();
        if raw.len() < 2 {
            return None;
        }
        Some(Self {
            shape,
            stops: fill_positions(&raw),
        })
    }

    /// The colour partway along, from 0 to 1.
    #[must_use]
    pub fn sample(&self, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let first = self.stops[0];
        if t <= first.0 {
            return first.1;
        }
        for pair in self.stops.windows(2) {
            let ((a, ca), (b, cb)) = (pair[0], pair[1]);
            if t <= b {
                let local = if b > a { (t - a) / (b - a) } else { 1.0 };
                return std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * local);
            }
        }
        self.stops[self.stops.len() - 1].1
    }
}

/// Splits on commas that aren't inside brackets.
fn split_arguments(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (index, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
}

fn parse_direction(text: &str) -> Option<f32> {
    let text = text.trim().to_ascii_lowercase();
    use std::f32::consts::PI;
    if let Some(degrees) = text.strip_suffix("deg") {
        return degrees.trim().parse::<f32>().ok().map(f32::to_radians);
    }
    if let Some(turns) = text.strip_suffix("turn") {
        return turns.trim().parse::<f32>().ok().map(|turn| turn * 2.0 * PI);
    }
    Some(match text.strip_prefix("to ")?.trim() {
        "top" => 0.0,
        "right" => PI / 2.0,
        "bottom" => PI,
        "left" => PI * 1.5,
        "top right" | "right top" => PI / 4.0,
        "bottom right" | "right bottom" => PI * 0.75,
        "bottom left" | "left bottom" => PI * 1.25,
        "top left" | "left top" => PI * 1.75,
        _ => return None,
    })
}

/// A colour at the start of the text, and whatever follows it.
fn split_color(text: &str) -> Option<(Rgba, &str)> {
    if let Some(open) = text.find('(') {
        let close = text.find(')')?;
        return Some((parse_color(&text[..=close])?, &text[close + 1..])).filter(|_| open < close);
    }
    let word = first_word(text);
    Some((parse_color(word)?, &text[word.len()..]))
}

/// Reads `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(...)`, `rgba(...)` and `transparent`.
#[must_use]
pub fn parse_color(text: &str) -> Option<Rgba> {
    let text = text.trim().to_ascii_lowercase();
    if text == "transparent" {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(hex) = text.strip_prefix('#') {
        let digit = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
        let short = |index: usize| {
            let value = u8::from_str_radix(hex.get(index..=index)?, 16).ok()?;
            Some(value * 17)
        };
        let (r, g, b, a) = match hex.len() {
            3 => (short(0)?, short(1)?, short(2)?, 255),
            6 => (digit(0)?, digit(2)?, digit(4)?, 255),
            8 => (digit(0)?, digit(2)?, digit(4)?, digit(6)?),
            _ => return None,
        };
        return Some([
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        ]);
    }
    let inner = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let values: Vec<f32> = inner
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    match values.as_slice() {
        [r, g, b] => Some([r / 255.0, g / 255.0, b / 255.0, 1.0]),
        [r, g, b, a] => Some([r / 255.0, g / 255.0, b / 255.0, *a]),
        _ => None,
    }
}

/// CSS gives unpositioned stops even spacing between the positioned ones around them.
fn fill_positions(raw: &[(Option<f32>, Rgba)]) -> Vec<(f32, Rgba)> {
    let mut positions: Vec<Option<f32>> = raw.iter().map(|(position, _)| *position).collect();
    let last = positions.len() - 1;
    positions[0] = positions[0].or(Some(0.0));
    positions[last] = positions[last].or(Some(1.0));
    let mut index = 1;
    while index < last {
        if positions[index].is_some() {
            index += 1;
            continue;
        }
        let end = (index..=last)
            .find(|&i| positions[i].is_some())
            .unwrap_or(last);
        let (from, to) = (
            positions[index - 1].unwrap_or(0.0),
            positions[end].unwrap_or(1.0),
        );
        let gaps = (end - index + 1) as f32;
        for (offset, slot) in positions[index..end].iter_mut().enumerate() {
            *slot = Some(from + (to - from) * (offset + 1) as f32 / gaps);
        }
        index = end;
    }
    let mut previous = 0.0_f32;
    positions
        .iter()
        .zip(raw)
        .map(|(position, (_, color))| {
            let position = position.unwrap_or(0.0).max(previous);
            previous = position;
            (position, *color)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_angle_is_honoured() {
        let gradient = Gradient::parse("linear-gradient(90deg, #000000, #ffffff)").unwrap();
        assert_eq!(gradient.shape, Shape::Linear { angle: PI / 2.0 });
    }

    #[test]
    fn directions_in_words_work() {
        let angle = |css| match Gradient::parse(css).unwrap().shape {
            Shape::Linear { angle } => angle,
            Shape::Radial => panic!("not linear"),
        };
        assert!(close(
            angle("linear-gradient(to right, #000, #fff)"),
            PI / 2.0
        ));
        assert!(close(
            angle("linear-gradient(to top left, #000, #fff)"),
            PI * 1.75
        ));
        assert!(
            close(angle("linear-gradient(#000, #fff)"), PI),
            "CSS defaults to top to bottom"
        );
        assert!(close(
            angle("linear-gradient(0.25turn, #000, #fff)"),
            PI / 2.0
        ));
    }

    #[test]
    fn stops_fill_in_like_css() {
        let gradient =
            Gradient::parse("linear-gradient(to right, #000, #808080 20%, #fff, #000)").unwrap();
        let positions: Vec<f32> = gradient
            .stops
            .iter()
            .map(|(position, _)| *position)
            .collect();
        assert_eq!(positions[0], 0.0);
        assert!(close(positions[1], 0.2));
        assert!(close(positions[2], 0.6));
        assert_eq!(positions[3], 1.0);
    }

    #[test]
    fn rgb_colours_with_commas_in_them_are_one_stop() {
        let gradient = Gradient::parse(
            "linear-gradient(to right, rgb(255, 0, 0) 0%, rgba(0, 0, 255, 0.5) 100%)",
        )
        .unwrap();
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[0].1, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(gradient.stops[1].1, [0.0, 0.0, 1.0, 0.5]);
    }

    #[test]
    fn radial_gradients_skip_their_shape() {
        let gradient = Gradient::parse("radial-gradient(circle at center, #000, #fff)").unwrap();
        assert_eq!(gradient.shape, Shape::Radial);
        assert_eq!(gradient.stops.len(), 2);
    }

    #[test]
    fn sampling_blends_between_stops() {
        let gradient = Gradient::parse("linear-gradient(to right, #000000, #ffffff)").unwrap();
        assert!(close(gradient.sample(0.5)[0], 0.5));
        assert_eq!(gradient.sample(-1.0)[0], 0.0);
        assert_eq!(gradient.sample(2.0)[0], 1.0);
    }

    #[test]
    fn nonsense_is_refused() {
        assert_eq!(Gradient::parse("not a gradient"), None);
        assert_eq!(Gradient::parse("linear-gradient(#000)"), None);
    }

    #[test]
    fn colours_parse_in_their_short_forms() {
        assert_eq!(parse_color("#fff"), Some([1.0, 1.0, 1.0, 1.0]));
        assert_eq!(parse_color("transparent"), Some([0.0, 0.0, 0.0, 0.0]));
        assert_eq!(parse_color("#12345"), None);
    }
}
