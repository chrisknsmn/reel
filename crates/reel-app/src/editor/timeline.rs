//! The timeline. T4 found plain page elements are quick enough, even at thousands of items, so
//! that's what it's built from.

use dioxus::prelude::*;
use reel_core::edit::{Edge, Edit};
use reel_core::history::Selection;
use reel_core::project::Project;
use reel_core::time::{Micros, TimelineTime};
use reel_core::timeline::{AudioItem, ItemId, Point, Timeline, ZoomMode};

use super::math::{View, ruler_ticks, snap_to_keyframes};
use super::state::{Ed, use_ed};
use crate::preview;

const ZOOM_GHOST: Micros = Micros(1_000_000);
const CAPTION_GHOST: Micros = Micros(1_500_000);
/// How much one notch of the wheel changes the view's length.
const WHEEL_ZOOM: f64 = 0.0025;
/// Reports where the timeline is, whenever it moves or changes size.
const OBSERVE: &str = r"
const start = () => {
    const el = document.getElementById('timeline-rows');
    if (!el) { setTimeout(start, 20); return; }
    const send = () => {
        const r = el.getBoundingClientRect();
        dioxus.send([r.x, r.y, r.width, r.height]);
    };
    new ResizeObserver(send).observe(el);
    addEventListener('resize', send);
    send();
};
start();
await new Promise(() => {});
";

/// Something on the timeline that can be dragged.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Clip(ItemId),
    Zoom(ItemId),
    Annotation(ItemId),
    Audio(ItemId),
    Caption(ItemId),
}

impl Target {
    fn selection(&self) -> Selection {
        match self {
            Self::Clip(id) => Selection::Clip(id.clone()),
            Self::Zoom(id) => Selection::Zoom(id.clone()),
            Self::Annotation(id) => Selection::Annotation(id.clone()),
            Self::Audio(id) => Selection::Audio(id.clone()),
            Self::Caption(id) => Selection::Caption(id.clone()),
        }
    }

    fn id(&self) -> &ItemId {
        match self {
            Self::Clip(id)
            | Self::Zoom(id)
            | Self::Annotation(id)
            | Self::Audio(id)
            | Self::Caption(id) => id,
        }
    }
}

/// What part of it is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Body,
    Start,
    End,
}

/// A drag in progress.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    target: Target,
    grip: Grip,
    /// How far into the item it was grabbed, so it doesn't jump to the pointer.
    offset: Micros,
    gesture: String,
}

/// What a drag asks the timeline to do.
#[must_use]
pub fn edit_for(target: &Target, grip: Grip, to: TimelineTime) -> Edit {
    let edge = if grip == Grip::Start {
        Edge::Start
    } else {
        Edge::End
    };
    match (target, grip) {
        (Target::Clip(id), Grip::Body) => Edit::MoveClip { id: id.clone(), to },
        (Target::Clip(id), _) => Edit::TrimClip {
            id: id.clone(),
            edge,
            to,
        },
        (Target::Zoom(id), Grip::Body) => Edit::MoveZoom { id: id.clone(), to },
        (Target::Zoom(id), _) => Edit::ResizeZoom {
            id: id.clone(),
            edge,
            to,
        },
        (Target::Annotation(id), Grip::Body) => Edit::MoveAnnotation {
            id: id.clone(),
            to,
            track: None,
        },
        (Target::Annotation(id), _) => Edit::ResizeAnnotation {
            id: id.clone(),
            edge,
            to,
        },
        (Target::Audio(id), Grip::Body) => Edit::MoveAudio {
            id: id.clone(),
            to,
            track: None,
        },
        (Target::Audio(id), _) => Edit::ResizeAudio {
            id: id.clone(),
            edge,
            to,
        },
        (Target::Caption(id), Grip::Body) => Edit::MoveCaption { id: id.clone(), to },
        (Target::Caption(id), _) => Edit::ResizeCaption {
            id: id.clone(),
            edge,
            to,
        },
    }
}

/// Which annotation or audio rows have anything on them, in order.
#[must_use]
pub fn tracks_in_use(mut tracks: Vec<u32>) -> Vec<u32> {
    tracks.sort_unstable();
    tracks.dedup();
    tracks
}

/// Where a new zoom would go at a time, if it's allowed there.
#[must_use]
pub fn zoom_ghost_at(timeline: &Timeline, time: TimelineTime) -> Option<TimelineTime> {
    let mut trial = timeline.clone();
    trial.apply(&add_zoom_edit(time)).ok().map(|_| time)
}

/// A new zoom's defaults: 1.5×, automatic, centred.
#[must_use]
pub fn add_zoom_edit(start: TimelineTime) -> Edit {
    Edit::AddZoom {
        start,
        depth: 1,
        focus: Point { x: 0.5, y: 0.5 },
        mode: ZoomMode::Auto,
    }
}

/// How long a new caption can be at a time: up to a second and a half, or until the next one.
#[must_use]
pub fn caption_ghost_end(timeline: &Timeline, time: TimelineTime) -> Option<TimelineTime> {
    if timeline
        .captions
        .iter()
        .any(|caption| time >= caption.start && time < caption.end)
    {
        return None;
    }
    let total = timeline.duration();
    let next = timeline
        .captions
        .iter()
        .map(|caption| caption.start)
        .filter(|start| *start > time)
        .min();
    let limit = next.unwrap_or(TimelineTime::ZERO + total);
    let end = (time + CAPTION_GHOST).min(limit);
    ((end - time) >= reel_core::timeline::MIN_ITEM).then_some(end)
}

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    view: View,
    width: f64,
}

impl Layout {
    fn style(&self, start: TimelineTime, end: TimelineTime) -> String {
        let left = self.view.x_of(start, self.width);
        let width = self.view.width_of(end - start, self.width).max(2.0);
        format!("left: {left:.1}px; width: {width:.1}px")
    }

    fn visible(&self, start: TimelineTime, end: TimelineTime) -> bool {
        end >= self.view.start && start <= self.view.start + self.view.length
    }
}

#[component]
fn Item(
    target: Target,
    start: TimelineTime,
    end: TimelineTime,
    label: String,
    class: String,
    layout: Layout,
    selected: bool,
    drag: Signal<Option<Drag>>,
    rect: Signal<(f64, f64, f64, f64)>,
    children: Element,
) -> Element {
    let ed = use_ed();
    let begin = move |grip: Grip, event: MouseEvent, target: Target| {
        event.stop_propagation();
        let x = event.client_coordinates().x - rect().0;
        let grabbed = layout.view.time_at(x, layout.width);
        let offset = match grip {
            Grip::Body => grabbed - start,
            Grip::Start => grabbed - start,
            Grip::End => grabbed - end,
        };
        ed.select(target.selection());
        let mut dragging = ed.dragging;
        dragging.set(true);
        let mut drag = drag;
        drag.set(Some(Drag {
            gesture: format!("drag:{}:{grip:?}", target.id().0),
            target,
            grip,
            offset,
        }));
    };
    let id = format!("item-{}", target.id().0);
    let (body, left, right) = (target.clone(), target.clone(), target);
    rsx! {
        div {
            class: "tl-item {class}",
            id: "{id}",
            "data-selected": "{selected}",
            title: "{label}",
            style: "{layout.style(start, end)}",
            onmousedown: move |event| begin(Grip::Body, event, body.clone()),
            div {
                class: "tl-cap tl-cap-start",
                onmousedown: move |event| begin(Grip::Start, event, left.clone()),
            }
            {children}
            span { class: "tl-label", "{label}" }
            div {
                class: "tl-cap tl-cap-end",
                onmousedown: move |event| begin(Grip::End, event, right.clone()),
            }
        }
    }
}

/// A waveform drawn from the sound's peaks, for the part of the file an item plays.
#[component]
fn Waveform(path: String, from: f64, to: f64) -> Element {
    let peaks = preview::peaks(std::path::Path::new(&path));
    if peaks.is_empty() {
        return rsx! {};
    }
    let count = peaks.len() as f64;
    let first = (from.clamp(0.0, 1.0) * count) as usize;
    let last = ((to.clamp(0.0, 1.0) * count) as usize).clamp(first + 1, peaks.len());
    let shown = &peaks[first.min(peaks.len() - 1)..last];
    let mut path_data = String::with_capacity(shown.len() * 12);
    for (index, peak) in shown.iter().enumerate() {
        let height = f64::from(*peak).clamp(0.0, 1.0) * 50.0;
        path_data.push_str(&format!(
            "M{index} {:.1}V{:.1}",
            50.0 - height,
            50.0 + height
        ));
    }
    rsx! {
        svg {
            class: "tl-wave",
            view_box: "0 0 {shown.len()} 100",
            preserve_aspect_ratio: "none",
            "aria-hidden": "true",
            path { d: "{path_data}", vector_effect: "non-scaling-stroke" }
        }
    }
}

fn file_name_of(item: &AudioItem) -> String {
    item.file.absolute.file_name().map_or_else(
        || "Audio".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[component]
pub fn TimelinePanel() -> Element {
    let ed = use_ed();
    let mut rect = use_signal(|| (0.0, 0.0, 1000.0, 200.0));
    let mut drag = use_signal(|| None::<Drag>);
    let mut scrubbing = use_signal(|| false);
    let mut zoom_ghost = use_signal(|| None::<TimelineTime>);
    let mut caption_ghost = use_signal(|| None::<(TimelineTime, TimelineTime)>);

    use_future(move || async move {
        let mut reports = document::eval(OBSERVE);
        while let Ok(value) = reports.recv::<serde_json::Value>().await {
            let number = |index: usize| value[index].as_f64().unwrap_or(0.0);
            rect.set((number(0), number(1), number(2).max(1.0), number(3)));
        }
    });

    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let total = project.timeline.duration();
    let view = (ed.view)();
    let width = rect().2;
    let layout = Layout { view, width };
    let selection = ed.selection();

    let on_wheel = move |event: WheelEvent| {
        let mut view_signal = ed.view;
        let delta = event.delta().strip_units();
        let modifiers = event.modifiers();
        let (x, _) = {
            let at = event.client_coordinates();
            (at.x - rect().0, at.y)
        };
        let current = view_signal();
        if modifiers.meta() || modifiers.ctrl() {
            event.prevent_default();
            view_signal.set(current.zoomed((delta.y * WHEEL_ZOOM).exp(), x, width, total));
        } else {
            let along = if modifiers.shift() {
                delta.y
            } else {
                delta.x + delta.y
            };
            view_signal.set(current.panned(along, width, total));
        }
    };

    let scrub_to = move |x: f64| {
        let time = (ed.view)().time_at(x, rect().2);
        let time = snap_to_keyframes(time, &(ed.keyframes)());
        let clamped = TimelineTime(time.0.clamp(0, total.0));
        ed.seek(clamped);
        let mut view_signal = ed.view;
        let next = view_signal().following(clamped, total);
        if next != view_signal() {
            view_signal.set(next);
        }
    };

    let tracks_of = |list: Vec<u32>| tracks_in_use(list);
    let annotation_rows = tracks_of(
        project
            .timeline
            .annotations
            .iter()
            .map(|item| item.track)
            .collect(),
    );
    let audio_rows = tracks_of(
        project
            .timeline
            .audio
            .iter()
            .map(|item| item.track)
            .collect(),
    );
    let show_captions = project.caption_style.show || !project.timeline.captions.is_empty();
    let hover_captions = project.caption_style.hover_to_add;
    let timeline_for_zoom = project.timeline.clone();
    let timeline_for_caption = project.timeline.clone();
    let source_rows = if project
        .timeline
        .clips
        .iter()
        .any(|clip| clip.show_source_audio)
    {
        project.media.source_tracks.clone()
    } else {
        Vec::new()
    };
    let folder = ed.folder();

    rsx! {
        div {
            class: "tl",
            id: "timeline",
            onwheel: on_wheel,
            // Ruler: click or drag anywhere to move the playhead.
            div {
                class: "tl-ruler",
                id: "timeline-ruler",
                onmousedown: move |event| {
                    scrubbing.set(true);
                    scrub_to(event.client_coordinates().x - rect().0);
                },
                for tick in ruler_ticks(&view) {
                    {
                        let left = view.x_of(tick.time, width);
                        let class = if tick.label.is_some() { "tl-tick tl-tick-major" } else { "tl-tick" };
                        rsx! {
                            div {
                                class,
                                style: "left: {left:.1}px",
                                if let Some(text) = tick.label { span { "{text}" } }
                            }
                        }
                    }
                }
                for key in (ed.keyframes)() {
                    div {
                        class: "tl-keyframe",
                        style: "left: {view.x_of(key, width):.1}px",
                        "aria-label": "Keyframe",
                    }
                }
            }
            div {
                class: "tl-rows",
                id: "timeline-rows",
                onmousedown: move |event| {
                    // An empty area clears the selection and seeks.
                    ed.select(Selection::None);
                    scrub_to(event.client_coordinates().x - rect().0);
                },
                // Clips.
                div { class: "tl-row", id: "row-clips",
                    for clip in project.timeline.clips.iter().filter(|clip| layout.visible(clip.start, clip.end())) {
                        Item {
                            key: "{clip.id.0}",
                            target: Target::Clip(clip.id.clone()),
                            start: clip.start,
                            end: clip.end(),
                            label: if (clip.speed - 1.0).abs() > f64::EPSILON { format!("Clip {}×", clip.speed) } else { "Clip".to_string() },
                            class: if clip.muted { "tl-clip tl-muted" } else { "tl-clip" },
                            layout,
                            selected: selection == Selection::Clip(clip.id.clone()),
                            drag,
                            rect,
                        }
                    }
                }
                // Source audio, only when a clip asks for it.
                for track in source_rows {
                    {
                        let path = track.file.as_ref().map_or_else(
                            || project.media.video.resolve(folder.as_deref()),
                            |file| file.resolve(folder.as_deref()),
                        ).to_string_lossy().into_owned();
                        let source_total = project.timeline.source_duration.secs().max(0.001);
                        rsx! {
                            div { class: "tl-row tl-row-source",
                                for clip in project.timeline.clips.iter().filter(|clip| clip.show_source_audio) {
                                    div {
                                        class: "tl-item tl-source",
                                        style: "{layout.style(clip.start, clip.end())}",
                                        Waveform {
                                            path: path.clone(),
                                            from: (clip.source_start - reel_core::time::SourceTime::ZERO).secs() / source_total,
                                            to: (clip.source_end() - reel_core::time::SourceTime::ZERO).secs() / source_total,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Zooms, with a ghost showing where one could go.
                div {
                    class: "tl-row",
                    id: "row-zooms",
                    onmousemove: move |event| {
                        let x = event.client_coordinates().x - rect().0;
                        let time = (ed.view)().time_at(x, rect().2);
                        zoom_ghost.set(zoom_ghost_at(&timeline_for_zoom, time));
                    },
                    onmouseleave: move |_| zoom_ghost.set(None),
                    for zoom in project.timeline.zooms.iter().filter(|zoom| layout.visible(zoom.start, zoom.end)) {
                        Item {
                            key: "{zoom.id.0}",
                            target: Target::Zoom(zoom.id.clone()),
                            start: zoom.start,
                            end: zoom.end,
                            label: format!("{}×", zoom.scale()),
                            class: if zoom.mode == ZoomMode::Manual { "tl-zoom tl-zoom-manual" } else { "tl-zoom" },
                            layout,
                            selected: selection == Selection::Zoom(zoom.id.clone()) || selection == Selection::AllZooms,
                            drag,
                            rect,
                        }
                    }
                    if let (Some(start), None) = (zoom_ghost(), drag()) {
                        div {
                            class: "tl-ghost",
                            id: "zoom-ghost",
                            style: "{layout.style(start, start + ZOOM_GHOST)}",
                            onmousedown: move |event| {
                                event.stop_propagation();
                                if let Some(outcome) = ed.apply(&add_zoom_edit(start))
                                    && let Some(id) = outcome.created
                                {
                                    ed.select(Selection::Zoom(id));
                                }
                                zoom_ghost.set(None);
                            },
                            "+"
                        }
                    }
                }
                // Captions, when there are any or they're turned on.
                if show_captions {
                    div {
                        class: "tl-row",
                        id: "row-captions",
                        onmousemove: move |event| {
                            if !hover_captions {
                                return;
                            }
                            let x = event.client_coordinates().x - rect().0;
                            let time = (ed.view)().time_at(x, rect().2);
                            caption_ghost.set(caption_ghost_end(&timeline_for_caption, time).map(|end| (time, end)));
                        },
                        onmouseleave: move |_| caption_ghost.set(None),
                        for caption in project.timeline.captions.iter().filter(|caption| layout.visible(caption.start, caption.end)) {
                            Item {
                                key: "{caption.id.0}",
                                target: Target::Caption(caption.id.clone()),
                                start: caption.start,
                                end: caption.end,
                                label: caption.text.clone(),
                                class: "tl-caption",
                                layout,
                                selected: selection == Selection::Caption(caption.id.clone()),
                                drag,
                                rect,
                            }
                        }
                        if let (Some((start, end)), None) = (caption_ghost(), drag()) {
                            div {
                                class: "tl-ghost tl-ghost-caption",
                                style: "{layout.style(start, end)}",
                                onmousedown: move |event| {
                                    event.stop_propagation();
                                    if let Some(outcome) = ed.apply(&Edit::AddCaption { start, end, text: String::new() })
                                        && let Some(id) = outcome.created
                                    {
                                        ed.select(Selection::Caption(id));
                                    }
                                    caption_ghost.set(None);
                                },
                                "+"
                            }
                        }
                    }
                }
                // Annotations, one row per track in use.
                for track in annotation_rows {
                    div { class: "tl-row", key: "annotations-{track}",
                        for item in project.timeline.annotations.iter().filter(|item| item.track == track && layout.visible(item.start, item.end)) {
                            Item {
                                key: "{item.id.0}",
                                target: Target::Annotation(item.id.clone()),
                                start: item.start,
                                end: item.end,
                                label: annotation_label(item),
                                class: "tl-annotation",
                                layout,
                                selected: selection == Selection::Annotation(item.id.clone()),
                                drag,
                                rect,
                            }
                        }
                    }
                }
                // Added sound, one row per track in use.
                for track in audio_rows {
                    div { class: "tl-row", key: "audio-{track}",
                        for item in project.timeline.audio.iter().filter(|item| item.track == track && layout.visible(item.start, item.end)) {
                            {
                                let path = item.file.resolve(folder.as_deref()).to_string_lossy().into_owned();
                                let file_seconds = item.file_duration.secs().max(0.001);
                                let played = (item.end - item.start).secs() / file_seconds;
                                rsx! {
                                    Item {
                                        key: "{item.id.0}",
                                        target: Target::Audio(item.id.clone()),
                                        start: item.start,
                                        end: item.end,
                                        label: file_name_of(item),
                                        class: "tl-audio",
                                        layout,
                                        selected: selection == Selection::Audio(item.id.clone()),
                                        drag,
                                        rect,
                                        Waveform { path, from: 0.0, to: played }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Playhead { view, width, scrubbing }
            // While dragging, this catches the mouse everywhere, even off the timeline.
            if drag().is_some() || scrubbing() {
                div {
                    class: "tl-capture",
                    id: "timeline-capture",
                    onmousemove: move |event| {
                        let x = event.client_coordinates().x - rect().0;
                        if scrubbing() {
                            scrub_to(x);
                        } else if let Some(active) = drag() {
                            continue_drag(ed, &active, x, rect().2);
                        }
                    },
                    onmouseup: move |_| {
                        scrubbing.set(false);
                        drag.set(None);
                        let mut dragging = ed.dragging;
                        dragging.set(false);
                        ed.end_gesture();
                    },
                }
            }
        }
    }
}

fn continue_drag(ed: Ed, active: &Drag, x: f64, width: f64) {
    let pointer = (ed.view)().time_at(x, width);
    let to = TimelineTime((pointer.0 - active.offset.0).max(0));
    let edit = edit_for(&active.target, active.grip, to);
    ed.change(&active.gesture, |project: &mut Project| {
        // A move that isn't allowed just doesn't happen: the item stays at the last place it could go.
        let _ = project.timeline.apply(&edit);
    });
    // Near the edges, the view scrolls along.
    let mut view = ed.view;
    let current = view();
    let next = current.following(TimelineTime(pointer.0.max(0)), ed.total());
    if next != current {
        view.set(next);
    }
}

fn annotation_label(item: &reel_core::timeline::Annotation) -> String {
    use reel_core::timeline::AnnotationContent as Content;
    match &item.content {
        Content::Text { text, .. } if !text.is_empty() => text.clone(),
        Content::Text { .. } => "Text".to_string(),
        Content::Image { .. } => "Image".to_string(),
        Content::Arrow { .. } => "Arrow".to_string(),
        Content::Blur { .. } => "Blur".to_string(),
    }
}

/// On its own, so that moving it doesn't redraw every item.
#[component]
fn Playhead(view: View, width: f64, scrubbing: Signal<bool>) -> Element {
    let ed = use_ed();
    let now = TimelineTime::ZERO + (ed.position)();
    let x = view.x_of(now, width);
    rsx! {
        div {
            class: "tl-playhead",
            id: "playhead",
            style: "transform: translateX({x:.1}px)",
            onmousedown: move |event| {
                event.stop_propagation();
                scrubbing.set(true);
            },
            div { class: "tl-playhead-head" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: f64) -> TimelineTime {
        TimelineTime::from_ms(ms)
    }

    fn timeline(seconds: i64) -> Timeline {
        Timeline::for_recording(Micros::from_secs(seconds))
    }

    #[test]
    fn drags_become_the_right_edits() {
        let id = ItemId::from("a");
        assert!(matches!(
            edit_for(&Target::Clip(id.clone()), Grip::Body, at(5.0)),
            Edit::MoveClip { .. }
        ));
        assert!(matches!(
            edit_for(&Target::Clip(id.clone()), Grip::Start, at(5.0)),
            Edit::TrimClip {
                edge: Edge::Start,
                ..
            }
        ));
        assert!(matches!(
            edit_for(&Target::Zoom(id.clone()), Grip::End, at(5.0)),
            Edit::ResizeZoom {
                edge: Edge::End,
                ..
            }
        ));
        assert!(matches!(
            edit_for(&Target::Annotation(id.clone()), Grip::Body, at(5.0)),
            Edit::MoveAnnotation { .. }
        ));
        assert!(matches!(
            edit_for(&Target::Audio(id.clone()), Grip::Start, at(5.0)),
            Edit::ResizeAudio { .. }
        ));
        assert!(matches!(
            edit_for(&Target::Caption(id), Grip::Body, at(5.0)),
            Edit::MoveCaption { .. }
        ));
    }

    #[test]
    fn rows_only_exist_for_tracks_in_use() {
        assert_eq!(tracks_in_use(vec![2, 0, 2, 5]), [0, 2, 5]);
        assert!(tracks_in_use(vec![]).is_empty());
    }

    #[test]
    fn a_zoom_ghost_only_shows_where_a_zoom_can_go() {
        let mut timeline = timeline(10);
        assert_eq!(zoom_ghost_at(&timeline, at(2000.0)), Some(at(2000.0)));
        timeline.apply(&add_zoom_edit(at(2000.0))).unwrap();
        assert_eq!(
            zoom_ghost_at(&timeline, at(2500.0)),
            None,
            "not on top of another"
        );
        assert_eq!(
            zoom_ghost_at(&timeline, at(1500.0)),
            None,
            "nor without a second's room"
        );
        assert_eq!(
            zoom_ghost_at(&timeline, at(9800.0)),
            None,
            "nor too near the clip's end"
        );
    }

    #[test]
    fn a_caption_ghost_stops_at_the_next_caption_or_the_end() {
        let mut timeline = timeline(10);
        assert_eq!(caption_ghost_end(&timeline, at(1000.0)), Some(at(2500.0)));
        assert_eq!(caption_ghost_end(&timeline, at(9500.0)), Some(at(10_000.0)));
        timeline
            .apply(&Edit::AddCaption {
                start: at(3000.0),
                end: at(4000.0),
                text: "x".into(),
            })
            .unwrap();
        assert_eq!(caption_ghost_end(&timeline, at(2000.0)), Some(at(3000.0)));
        assert_eq!(
            caption_ghost_end(&timeline, at(3500.0)),
            None,
            "not on top of one"
        );
        assert_eq!(
            caption_ghost_end(&timeline, at(2950.0)),
            None,
            "nor with no room"
        );
    }
}
