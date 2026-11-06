//! How strong each zoom is at a given time (§32).

use reel_core::time::TimelineTime;
use reel_core::timeline::Zoom;

use crate::easing::{clamp01, ease_out_zoom};

/// The default zoom-out length, which the lead-in is measured against.
const TRANSITION_MS: f64 = 1015.05;
const ZOOM_IN_WINDOW_MS: f64 = TRANSITION_MS * 1.5;
const ZOOM_OUT_EARLY_MS: f64 = 500.0;
const ZOOM_IN_OVERLAP_MS: f64 = 1000.0;
const ANIMATION_LEAD_MS: f64 = 200.0;
/// Zooms this close together turn into one camera move.
const CONNECTED_GAP_MS: f64 = 1350.0;
const CONNECTED_PAN_MS: f64 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomOptions {
    pub zoom_in_ms: f64,
    pub zoom_out_ms: f64,
    pub connect: bool,
}

/// A zoom's strength at a time, from 0 to 1.
#[must_use]
pub fn strength(zoom: &Zoom, time_ms: f64, options: &ZoomOptions) -> f64 {
    let (start, end) = (zoom.start.ms(), zoom.end.ms());
    let zoom_in = options.zoom_in_ms.max(1.0);
    let zoom_out = options.zoom_out_ms.max(1.0);
    let adjusted = time_ms - ANIMATION_LEAD_MS;
    let lead_in = start + ZOOM_IN_OVERLAP_MS - ZOOM_IN_WINDOW_MS;
    let mut zoom_out_start = end - ZOOM_OUT_EARLY_MS;
    let mut zoom_in_end = lead_in + zoom_in;
    if zoom_in_end > zoom_out_start {
        let middle = f64::midpoint(zoom_in_end, zoom_out_start);
        zoom_in_end = middle;
        zoom_out_start = middle;
    }
    let lead_out_end = zoom_out_start + zoom_out;
    if adjusted < lead_in || adjusted > lead_out_end {
        return 0.0;
    }
    if adjusted < zoom_in_end {
        return ease_out_zoom((adjusted - lead_in) / zoom_in);
    }
    if adjusted <= zoom_out_start {
        return 1.0;
    }
    1.0 - ease_out_zoom(clamp01((adjusted - zoom_out_start) / zoom_out))
}

/// Two zooms that hand over to each other.
#[derive(Clone, Copy, Debug)]
struct Connection {
    current: usize,
    next: usize,
    start_ms: f64,
    end_ms: f64,
}

fn connections(zooms: &[Zoom]) -> Vec<Connection> {
    zooms
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[1].start.ms() - pair[0].end.ms() <= CONNECTED_GAP_MS)
        .map(|(index, pair)| Connection {
            current: index,
            next: index + 1,
            start_ms: pair[0].end.ms() + ANIMATION_LEAD_MS,
            end_ms: pair[0].end.ms() + ANIMATION_LEAD_MS + CONNECTED_PAN_MS,
        })
        .collect()
}

/// Which zoom is in charge at a time, and how strong it is. `zooms` must be sorted, as a timeline's
/// always are.
#[must_use]
pub fn dominant(zooms: &[Zoom], time: TimelineTime, options: &ZoomOptions) -> Option<(usize, f64)> {
    let time_ms = time.ms();
    let links = if options.connect { connections(zooms) } else { Vec::new() };

    for link in &links {
        if time_ms >= link.end_ms && time_ms < zooms[link.next].start.ms() {
            return Some((link.next, 1.0));
        }
    }

    let mut best: Option<(usize, f64)> = None;
    for (index, zoom) in zooms.iter().enumerate() {
        let value = connected_strength(zooms, index, zoom, time_ms, &links, options);
        if value <= 0.0 {
            continue;
        }
        // The strongest wins, and a later zoom wins a tie.
        let better = best.is_none_or(|(_, current)| value >= current);
        if better {
            best = Some((index, value));
        }
    }
    best
}

fn connected_strength(
    zooms: &[Zoom],
    index: usize,
    zoom: &Zoom,
    time_ms: f64,
    links: &[Connection],
    options: &ZoomOptions,
) -> f64 {
    if let Some(link) = links.iter().find(|link| link.current == index) {
        // The outgoing zoom holds full strength up to the handoff, then is gone.
        if time_ms >= link.start_ms {
            return 0.0;
        }
        if time_ms >= zooms[link.current].end.ms() - ZOOM_OUT_EARLY_MS + ANIMATION_LEAD_MS {
            return 1.0;
        }
    }
    if let Some(link) = links.iter().find(|link| link.next == index) {
        // The incoming zoom is at full strength from the handoff until its own zoom out.
        if time_ms < link.start_ms {
            return 0.0;
        }
        if time_ms < zooms[link.next].end.ms() - ZOOM_OUT_EARLY_MS + ANIMATION_LEAD_MS {
            return 1.0;
        }
    }
    strength(zoom, time_ms, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reel_core::timeline::{Point, ZoomMode};

    fn zoom(start: f64, end: f64) -> Zoom {
        Zoom {
            id: "z".into(),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            depth: 2,
            focus: Point::CENTER,
            mode: ZoomMode::Manual,
        }
    }

    const FOCUSED: ZoomOptions = ZoomOptions { zoom_in_ms: 200.0, zoom_out_ms: 200.0, connect: true };
    const SEPARATE: ZoomOptions = ZoomOptions { zoom_in_ms: 200.0, zoom_out_ms: 200.0, connect: false };

    #[test]
    fn strength_follows_the_documented_timing() {
        // §32 for a zoom from 5,000 to 11,000 ms with 200 ms eases:
        // t' = t − 200, leadIn = start + 1000 − 1522.575 = 4477.425, zoomInEnd = leadIn + 200,
        // zoomOutStart = end − 500 = 10,500, leadOutEnd = 10,700.
        let z = zoom(5000.0, 11_000.0);
        let at = |time| strength(&z, time, &SEPARATE);
        assert_eq!(at(4677.0), 0.0, "before the lead-in");
        let lead_in_shifted = 4477.425 + 200.0;
        assert!((at(lead_in_shifted + 100.0) - ease_out_zoom(0.5)).abs() < 1e-9);
        assert_eq!(at(lead_in_shifted + 200.0), 1.0);
        assert_eq!(at(10_700.0), 1.0, "still full just as the zoom out begins");
        assert!((at(10_800.0) - (1.0 - ease_out_zoom(0.5))).abs() < 1e-9);
        assert!(at(10_899.0) < 0.01);
        assert_eq!(at(10_901.0), 0.0, "after the lead-out");
    }

    #[test]
    fn a_short_zoom_meets_in_the_middle() {
        let z = zoom(5000.0, 5200.0);
        let options = ZoomOptions { zoom_in_ms: 2000.0, zoom_out_ms: 200.0, connect: false };
        let values: Vec<f64> = (0..300).map(|step| strength(&z, 4000.0 + f64::from(step) * 10.0, &options)).collect();
        assert!(values.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn nothing_is_dominant_when_no_zoom_is_active() {
        let zooms = [zoom(5000.0, 6000.0)];
        assert_eq!(dominant(&zooms, TimelineTime::from_ms(1000.0), &SEPARATE), None);
    }

    #[test]
    fn nearby_zooms_hand_over_with_no_dip() {
        // A 1,000 ms gap is within the 1,350 ms limit.
        let zooms = [zoom(5000.0, 8000.0), zoom(9000.0, 12_000.0)];
        for step in 0..=200 {
            let time = 7600.0 + f64::from(step) * 20.0;
            let (_, value) = dominant(&zooms, TimelineTime::from_ms(time), &FOCUSED).expect("a zoom should be active");
            assert_eq!(value, 1.0, "strength dipped at {time}");
        }
        assert_eq!(dominant(&zooms, TimelineTime::from_ms(8100.0), &FOCUSED).unwrap().0, 0);
        assert_eq!(dominant(&zooms, TimelineTime::from_ms(8300.0), &FOCUSED).unwrap().0, 1);
    }

    #[test]
    fn zooms_further_apart_zoom_out_in_between() {
        let zooms = [zoom(5000.0, 8000.0), zoom(10_000.0, 12_000.0)];
        assert_eq!(dominant(&zooms, TimelineTime::from_ms(9000.0), &FOCUSED), None);
    }

    #[test]
    fn with_connect_off_they_zoom_out_between_even_when_close() {
        let zooms = [zoom(5000.0, 8000.0), zoom(9200.0, 12_000.0)];
        let time = TimelineTime::from_ms(8400.0);
        assert_eq!(dominant(&zooms, time, &SEPARATE), None);
        assert!(dominant(&zooms, time, &FOCUSED).is_some());
    }
}
