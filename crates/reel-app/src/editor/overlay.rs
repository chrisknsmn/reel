//! What sits over the preview: invisible handles for picking and dragging things that wgpu draws
//! (BUILD_PLAN 7.5), the zoom's focus, and the crop editor.

use dioxus::prelude::*;
use reel_core::edit::Edit;
use reel_core::history::Selection;
use reel_core::project::Crop;
use reel_core::time::TimelineTime;
use reel_core::timeline::{ItemId, Point, ZoomMode};
use reel_motion::camera::Camera;
use reel_motion::layout::Rect;

use super::state::{Ed, Overlay, use_ed};
use crate::preview;

/// Where the stage shows inside the hole, in CSS pixels: the largest fit, centred.
#[must_use]
pub fn fitted(hole: (f64, f64, f64, f64), stage: (u32, u32)) -> Rect {
    let (_, _, width, height) = hole;
    let (stage_w, stage_h) = (f64::from(stage.0), f64::from(stage.1));
    let scale = (width / stage_w.max(1.0)).min(height / stage_h.max(1.0));
    let (shown_w, shown_h) = (stage_w * scale, stage_h * scale);
    Rect {
        x: (width - shown_w) / 2.0,
        y: (height - shown_h) / 2.0,
        width: shown_w,
        height: shown_h,
    }
}

/// Converts between places on the stage and places in the hole, through the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageMap {
    pub shown: Rect,
    pub stage: (f64, f64),
    pub camera: Camera,
}

impl StageMap {
    fn scale(&self) -> f64 {
        self.shown.width / self.stage.0.max(1.0)
    }

    /// A place on the stage, as a place in the hole.
    #[must_use]
    pub fn to_hole(self, x: f64, y: f64) -> (f64, f64) {
        let scale = self.scale();
        (
            self.shown.x + (x * self.camera.scale + self.camera.x * self.stage.0) * scale,
            self.shown.y + (y * self.camera.scale + self.camera.y * self.stage.1) * scale,
        )
    }

    /// A place in the hole, as a place on the stage.
    #[must_use]
    pub fn to_stage(self, x: f64, y: f64) -> (f64, f64) {
        let scale = self.scale().max(1e-9);
        (
            ((x - self.shown.x) / scale - self.camera.x * self.stage.0) / self.camera.scale,
            ((y - self.shown.y) / scale - self.camera.y * self.stage.1) / self.camera.scale,
        )
    }

    /// How many hole pixels a stage length covers.
    #[must_use]
    pub fn length(&self, length: f64) -> f64 {
        length * self.camera.scale * self.scale()
    }
}

/// Which part of an annotation's handle is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Body,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The smallest an annotation can be made on the preview, in percent of the frame.
const SMALLEST_PERCENT: f64 = 2.0;

/// An annotation's place after a drag by this many percent of the frame.
#[must_use]
pub fn dragged_place(grip: Grip, start: (Point, f64, f64), dx: f64, dy: f64) -> (Point, f64, f64) {
    let (position, width, height) = start;
    let right = position.x + width;
    let bottom = position.y + height;
    let (mut left_x, mut top_y, mut right_x, mut bottom_y) =
        (position.x, position.y, right, bottom);
    match grip {
        Grip::Body => {
            let x = (position.x + dx).clamp(-width + 1.0, 99.0);
            let y = (position.y + dy).clamp(-height + 1.0, 99.0);
            return (Point { x, y }, width, height);
        }
        Grip::TopLeft => {
            left_x = (left_x + dx).min(right - SMALLEST_PERCENT);
            top_y = (top_y + dy).min(bottom - SMALLEST_PERCENT);
        }
        Grip::TopRight => {
            right_x = (right_x + dx).max(position.x + SMALLEST_PERCENT);
            top_y = (top_y + dy).min(bottom - SMALLEST_PERCENT);
        }
        Grip::BottomLeft => {
            left_x = (left_x + dx).min(right - SMALLEST_PERCENT);
            bottom_y = (bottom_y + dy).max(position.y + SMALLEST_PERCENT);
        }
        Grip::BottomRight => {
            right_x = (right_x + dx).max(position.x + SMALLEST_PERCENT);
            bottom_y = (bottom_y + dy).max(position.y + SMALLEST_PERCENT);
        }
    }
    (
        Point {
            x: left_x,
            y: top_y,
        },
        right_x - left_x,
        bottom_y - top_y,
    )
}

#[derive(Clone, Debug, PartialEq)]
struct PlaceDrag {
    id: ItemId,
    grip: Grip,
    from_mouse: (f64, f64),
    start: (Point, f64, f64),
    frame: Rect,
    map: StageMap,
}

fn current_map(ed: &Ed) -> Option<(StageMap, Rect)> {
    let (stage, frame) = preview::stage_and_frame()?;
    let camera = preview::camera_at(ed.now()).unwrap_or(Camera::IDENTITY);
    Some((
        StageMap {
            shown: fitted((ed.hole)(), stage),
            stage: (f64::from(stage.0), f64::from(stage.1)),
            camera,
        },
        frame,
    ))
}

#[component]
pub fn PreviewOverlay() -> Element {
    let ed = use_ed();
    let mut drag = use_signal(|| None::<PlaceDrag>);
    let mut aiming = use_signal(|| false);
    let mut webcam_drag = use_signal(|| None::<((f64, f64), (f64, f64), (f64, f64), f64)>);
    // Reading these redraws the handles as the playhead moves and the hole changes.
    let _ = (ed.position)();
    let _ = (ed.hole)();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    if (ed.overlay)() == Overlay::Crop {
        return rsx! { CropEditor {} };
    }
    let Some((map, frame)) = current_map(&ed) else {
        return rsx! {};
    };
    let now = ed.now();
    let selection = ed.selection();
    let hole_left_top = ((ed.hole)().0, (ed.hole)().1);

    // A selected zoom on a fixed spot can be aimed by clicking or dragging on the preview.
    let aimed = match &selection {
        Selection::Zoom(id) => project
            .timeline
            .zooms
            .iter()
            .find(|zoom| zoom.id == *id && zoom.mode == ZoomMode::Manual)
            .cloned(),
        _ => None,
    };
    let aim = move |event: MouseEvent, id: ItemId| {
        let at = event.client_coordinates();
        let (stage_x, stage_y) = map.to_stage(at.x - hole_left_top.0, at.y - hole_left_top.1);
        let focus = Point {
            x: ((stage_x - frame.x) / frame.width.max(1.0)).clamp(0.0, 1.0),
            y: ((stage_y - frame.y) / frame.height.max(1.0)).clamp(0.0, 1.0),
        };
        let edit = Edit::UpdateZoom {
            id,
            depth: None,
            focus: Some(focus),
            mode: None,
        };
        ed.change("zoom-focus", |p| {
            let _ = p.timeline.apply(&edit);
        });
    };

    // A bubble that can be dragged around.
    let webcam_bubble = preview::webcam_size()
        .filter(|_| project.webcam.enabled)
        .map(|size| {
            let scale = (map.stage.0 / 1920.0).min(map.stage.1 / 1080.0);
            reel_motion::webcam::layout(
                &project.webcam,
                map.stage,
                (f64::from(size.0), f64::from(size.1)),
                map.camera.scale,
                scale,
            )
        });

    let active: Vec<_> = project
        .timeline
        .annotations
        .iter()
        .filter(|item| now >= item.start && now < item.end)
        .cloned()
        .collect();

    rsx! {
        div { class: "ph-layer", id: "preview-overlay",
            if let Some(zoom) = aimed {
                {
                    let (id_down, id_move) = (zoom.id.clone(), zoom.id.clone());
                    let fx = frame.x + zoom.focus.x * frame.width;
                    let fy = frame.y + zoom.focus.y * frame.height;
                    let (w, h) = (frame.width / zoom.scale(), frame.height / zoom.scale());
                    let (left, top) = map.to_hole(fx - w / 2.0, fy - h / 2.0);
                    rsx! {
                        div {
                            class: "ph-aim",
                            id: "zoom-aim",
                            onmousedown: move |event| {
                                aiming.set(true);
                                aim(event, id_down.clone());
                            },
                            onmousemove: move |event| {
                                if aiming() {
                                    aim(event, id_move.clone());
                                }
                            },
                            onmouseup: move |_| {
                                aiming.set(false);
                                ed.end_gesture();
                            },
                            onmouseleave: move |_| {
                                if aiming() {
                                    aiming.set(false);
                                    ed.end_gesture();
                                }
                            },
                            div {
                                class: "ph-focus",
                                style: "left: {left:.1}px; top: {top:.1}px; width: {map.length(w):.1}px; height: {map.length(h):.1}px",
                            }
                        }
                    }
                }
            }
            for item in active {
                {
                    let selected = selection == Selection::Annotation(item.id.clone());
                    let x = frame.x + item.position.x / 100.0 * frame.width;
                    let y = frame.y + item.position.y / 100.0 * frame.height;
                    let (left, top) = map.to_hole(x, y);
                    let width = map.length(item.width / 100.0 * frame.width);
                    let height = map.length(item.height / 100.0 * frame.height);
                    let id = item.id.clone();
                    let start = (item.position, item.width, item.height);
                    let mut grab = move |event: MouseEvent, grip: Grip, id: ItemId| {
                        event.stop_propagation();
                        ed.select(Selection::Annotation(id.clone()));
                        let at = event.client_coordinates();
                        drag.set(Some(PlaceDrag { id, grip, from_mouse: (at.x, at.y), start, frame, map }));
                    };
                    let id_body = id.clone();
                    rsx! {
                        div {
                            key: "{id.0}",
                            class: "ph-annotation",
                            "data-selected": "{selected}",
                            style: "left: {left:.1}px; top: {top:.1}px; width: {width:.1}px; height: {height:.1}px",
                            onmousedown: move |event| grab(event, Grip::Body, id_body.clone()),
                            if selected {
                                for (grip, name) in [(Grip::TopLeft, "tl"), (Grip::TopRight, "tr"), (Grip::BottomLeft, "bl"), (Grip::BottomRight, "br")] {
                                    {
                                        let id = id.clone();
                                        rsx! {
                                            div {
                                                class: "ph-grip ph-grip-{name}",
                                                onmousedown: move |event| grab(event, grip, id.clone()),
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let (Some(bubble), true) = (webcam_bubble, (ed.rail)() == super::state::Rail::Webcam) {
                {
                    let (left, top) = (map.shown.x + bubble.frame.x * map.scale(), map.shown.y + bubble.frame.y * map.scale());
                    let (width, height) = (bubble.frame.width * map.scale(), bubble.frame.height * map.scale());
                    let from = (bubble.frame.x, bubble.frame.y);
                    let size = (bubble.frame.width, bubble.frame.height);
                    let margin = project.webcam.margin * (map.stage.0 / 1920.0).min(map.stage.1 / 1080.0);
                    rsx! {
                        div {
                            class: "ph-annotation ph-webcam",
                            id: "webcam-handle",
                            title: "Drag to move the webcam",
                            style: "left: {left:.1}px; top: {top:.1}px; width: {width:.1}px; height: {height:.1}px",
                            onmousedown: move |event| {
                                event.stop_propagation();
                                let at = event.client_coordinates();
                                webcam_drag.set(Some(((at.x, at.y), from, size, margin)));
                            },
                        }
                    }
                }
            }
            if let Some((grabbed, from, size, margin)) = webcam_drag() {
                div {
                    class: "ph-capture",
                    onmousemove: move |event| {
                        let at = event.client_coordinates();
                        let scale = map.scale().max(1e-9);
                        let x = from.0 + (at.x - grabbed.0) / scale;
                        let y = from.1 + (at.y - grabbed.1) / scale;
                        let (custom_x, custom_y) = webcam_position_for(x, y, size, map.stage, margin);
                        ed.change("webcam-drag", |p| {
                            p.webcam.position = reel_core::project::WebcamPosition::Custom;
                            p.webcam.custom_x = custom_x;
                            p.webcam.custom_y = custom_y;
                        });
                    },
                    onmouseup: move |_| {
                        webcam_drag.set(None);
                        ed.end_gesture();
                    },
                }
            }
            if let Some(active) = drag() {
                div {
                    class: "ph-capture",
                    onmousemove: move |event| {
                        let at = event.client_coordinates();
                        let to_percent = |pixels: f64, size: f64| pixels / active.map.length(size).max(1.0) * 100.0;
                        let dx = to_percent(at.x - active.from_mouse.0, active.frame.width);
                        let dy = to_percent(at.y - active.from_mouse.1, active.frame.height);
                        let (position, width, height) = dragged_place(active.grip, active.start, dx, dy);
                        let edit = Edit::PlaceAnnotation { id: active.id.clone(), position, width, height };
                        ed.change("annotation-place", |p| {
                            let _ = p.timeline.apply(&edit);
                        });
                    },
                    onmouseup: move |_| {
                        drag.set(None);
                        ed.end_gesture();
                    },
                }
            }
        }
    }
}

/// A webcam bubble's custom position, from where its top-left corner has been dragged to on the stage.
/// It's the opposite of how the position is laid out, so it comes out the same whatever the stage's size.
#[must_use]
pub fn webcam_position_for(
    x: f64,
    y: f64,
    size: (f64, f64),
    stage: (f64, f64),
    margin: f64,
) -> (f64, f64) {
    let fraction = |at: f64, length: f64, room: f64| {
        let travel = room - length - 2.0 * margin;
        if travel <= 0.0 {
            0.0
        } else {
            ((at - margin) / travel).clamp(0.0, 1.0)
        }
    };
    (fraction(x, size.0, stage.0), fraction(y, size.1, stage.1))
}

/// The edges of a crop being edited, as fractions of the video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropGrip {
    Top,
    Bottom,
    Left,
    Right,
}

const SMALLEST_CROP_DRAG: f64 = 0.1;

/// A crop after dragging one of its edges to a place, as a fraction of the video.
#[must_use]
pub fn dragged_crop(crop: Crop, grip: CropGrip, to: f64) -> Crop {
    let mut next = crop;
    let to = to.clamp(0.0, 1.0);
    match grip {
        CropGrip::Left => {
            let right = crop.x + crop.width;
            next.x = to.min(right - SMALLEST_CROP_DRAG);
            next.width = right - next.x;
        }
        CropGrip::Right => {
            next.width = (to - crop.x).max(SMALLEST_CROP_DRAG);
        }
        CropGrip::Top => {
            let bottom = crop.y + crop.height;
            next.y = to.min(bottom - SMALLEST_CROP_DRAG);
            next.height = bottom - next.y;
        }
        CropGrip::Bottom => {
            next.height = (to - crop.y).max(SMALLEST_CROP_DRAG);
        }
    }
    next
}

/// Shows the whole video with its crop on top. Done keeps it, as a single undo step, and Cancel puts
/// things back (BUILD_PLAN 7.1).
#[component]
fn CropEditor() -> Element {
    let mut ed = use_ed();
    let Some(project) = ed.project() else {
        return rsx! {};
    };
    let original = project.look.crop;
    let mut draft = use_signal(move || original);
    let mut dragging = use_signal(|| None::<CropGrip>);

    // While editing, the preview shows the video uncropped, so there's something to crop from.
    let uncropped = {
        let mut copy = project.clone();
        copy.look.crop = Crop::default();
        // And without the zoom, so the box lines up with the picture wherever the playhead is.
        copy.timeline.zooms.clear();
        copy
    };
    use_hook(|| preview::update(uncropped.clone()));
    let restore = project.clone();

    let Some((stage, _)) = preview::stage_and_frame() else {
        return rsx! {};
    };
    let Some(video) = preview::video_size() else {
        return rsx! {};
    };
    let map = StageMap {
        shown: fitted((ed.hole)(), stage),
        stage: (f64::from(stage.0), f64::from(stage.1)),
        camera: Camera::IDENTITY,
    };
    let frame = reel_motion::layout::compute(
        map.stage,
        &project.look.padding,
        &Crop::default(),
        (f64::from(video.0), f64::from(video.1)),
    )
    .frame;
    let crop = draft();
    let (left, top) = map.to_hole(
        frame.x + crop.x * frame.width,
        frame.y + crop.y * frame.height,
    );
    let (width, height) = (
        map.length(crop.width * frame.width),
        map.length(crop.height * frame.height),
    );
    let hole = (ed.hole)();
    rsx! {
        div {
            class: "ph-crop",
            id: "crop-editor",
            onmousemove: move |event| {
                let Some(grip) = dragging() else { return };
                let at = event.client_coordinates();
                let (x, y) = map.to_stage(at.x - hole.0, at.y - hole.1);
                let to = match grip {
                    CropGrip::Left | CropGrip::Right => (x - frame.x) / frame.width.max(1.0),
                    CropGrip::Top | CropGrip::Bottom => (y - frame.y) / frame.height.max(1.0),
                };
                draft.set(dragged_crop(draft(), grip, to));
            },
            onmouseup: move |_| dragging.set(None),
            div {
                class: "ph-crop-box",
                id: "crop-box",
                style: "left: {left:.1}px; top: {top:.1}px; width: {width:.1}px; height: {height:.1}px",
                for (grip, name) in [(CropGrip::Top, "top"), (CropGrip::Bottom, "bottom"), (CropGrip::Left, "left"), (CropGrip::Right, "right")] {
                    div {
                        class: "ph-crop-edge ph-crop-{name}",
                        id: "crop-{name}",
                        onmousedown: move |event| {
                            event.stop_propagation();
                            dragging.set(Some(grip));
                        },
                    }
                }
            }
            div { class: "ph-crop-actions",
                button {
                    class: "row-button",
                    id: "crop-cancel",
                    onclick: move |_| {
                        preview::update(restore.clone());
                        ed.overlay.set(Overlay::None);
                    },
                    "Cancel"
                }
                button {
                    class: "row-button row-primary",
                    id: "crop-done",
                    onclick: move |_| {
                        let chosen = draft();
                        ed.change("crop-dialog", |p| p.look.crop = chosen);
                        ed.end_gesture();
                        if let Some(current) = ed.project() {
                            preview::update(current);
                        }
                        ed.overlay.set(Overlay::None);
                    },
                    "Done"
                }
            }
        }
    }
}

/// The annotation under the playhead after this one, for Tab.
#[must_use]
pub fn cycle_annotation(
    annotations: &[reel_core::timeline::Annotation],
    now: TimelineTime,
    selected: Option<&ItemId>,
    backward: bool,
) -> Option<ItemId> {
    let under: Vec<&ItemId> = annotations
        .iter()
        .filter(|item| now >= item.start && now < item.end)
        .map(|item| &item.id)
        .collect();
    if under.is_empty() {
        return None;
    }
    let count = under.len();
    let index = selected
        .and_then(|id| under.iter().position(|other| *other == id))
        .map_or(0, |at| {
            if backward {
                (at + count - 1) % count
            } else {
                (at + 1) % count
            }
        });
    Some(under[index].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> StageMap {
        StageMap {
            shown: Rect {
                x: 100.0,
                y: 0.0,
                width: 960.0,
                height: 540.0,
            },
            stage: (1920.0, 1080.0),
            camera: Camera::IDENTITY,
        }
    }

    #[test]
    fn the_stage_fits_inside_the_hole_and_is_centred() {
        let shown = fitted((0.0, 0.0, 1160.0, 540.0), (1920, 1080));
        assert_eq!(
            (shown.x, shown.y, shown.width, shown.height),
            (100.0, 0.0, 960.0, 540.0)
        );
        let tall = fitted((0.0, 0.0, 400.0, 800.0), (1920, 1080));
        assert_eq!((tall.width, tall.height), (400.0, 225.0));
        assert_eq!(tall.y, 287.5);
    }

    #[test]
    fn places_go_there_and_back() {
        let plain = map();
        assert_eq!(plain.to_hole(0.0, 0.0), (100.0, 0.0));
        assert_eq!(plain.to_hole(1920.0, 1080.0), (1060.0, 540.0));
        let zoomed = StageMap {
            camera: Camera {
                scale: 2.0,
                x: -0.25,
                y: -0.5,
            },
            ..plain
        };
        let (x, y) = zoomed.to_hole(700.0, 300.0);
        let (back_x, back_y) = zoomed.to_stage(x, y);
        assert!((back_x - 700.0).abs() < 1e-6 && (back_y - 300.0).abs() < 1e-6);
    }

    #[test]
    fn lengths_grow_with_the_zoom() {
        let plain = map();
        assert_eq!(plain.length(1920.0), 960.0);
        let zoomed = StageMap {
            camera: Camera {
                scale: 2.0,
                x: 0.0,
                y: 0.0,
            },
            ..plain
        };
        assert_eq!(zoomed.length(1920.0), 1920.0);
    }

    fn start() -> (Point, f64, f64) {
        (Point { x: 40.0, y: 40.0 }, 20.0, 20.0)
    }

    #[test]
    fn dragging_the_body_moves_it_and_keeps_its_size() {
        let (position, width, height) = dragged_place(Grip::Body, start(), 10.0, -5.0);
        assert_eq!(
            (position.x, position.y, width, height),
            (50.0, 35.0, 20.0, 20.0)
        );
    }

    #[test]
    fn dragging_a_corner_keeps_the_opposite_one_still() {
        let (position, width, height) = dragged_place(Grip::BottomRight, start(), 10.0, 5.0);
        assert_eq!(
            (position.x, position.y, width, height),
            (40.0, 40.0, 30.0, 25.0)
        );
        let (position, width, height) = dragged_place(Grip::TopLeft, start(), -10.0, -5.0);
        assert_eq!((position.x, position.y), (30.0, 35.0));
        assert_eq!((position.x + width, position.y + height), (60.0, 60.0));
    }

    #[test]
    fn an_annotation_cant_be_made_too_small_or_inside_out() {
        let (_, width, height) = dragged_place(Grip::BottomRight, start(), -100.0, -100.0);
        assert_eq!((width, height), (SMALLEST_PERCENT, SMALLEST_PERCENT));
        let (position, width, _) = dragged_place(Grip::TopLeft, start(), 100.0, 0.0);
        assert_eq!(position.x + width, 60.0);
        assert_eq!(width, SMALLEST_PERCENT);
    }

    #[test]
    fn it_cant_be_dragged_right_off_the_frame() {
        let (position, _, _) = dragged_place(Grip::Body, start(), 1000.0, 1000.0);
        assert_eq!((position.x, position.y), (99.0, 99.0));
        let (position, _, _) = dragged_place(Grip::Body, start(), -1000.0, -1000.0);
        assert_eq!((position.x, position.y), (-19.0, -19.0));
    }

    #[test]
    fn a_dragged_webcam_lands_where_it_was_put() {
        let stage = (1920.0, 1080.0);
        let size = (400.0, 400.0);
        assert_eq!(
            webcam_position_for(24.0, 24.0, size, stage, 24.0),
            (0.0, 0.0)
        );
        let (x, y) = webcam_position_for(1496.0, 656.0, size, stage, 24.0);
        assert!((x - 1.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9);
        let (x, _) = webcam_position_for(760.0, 0.0, size, stage, 24.0);
        assert!((x - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_dragged_webcam_stays_on_the_stage() {
        assert_eq!(
            webcam_position_for(-500.0, 9000.0, (400.0, 400.0), (1920.0, 1080.0), 24.0),
            (0.0, 1.0)
        );
    }

    #[test]
    fn a_webcam_too_big_to_move_doesnt_divide_by_nothing() {
        assert_eq!(
            webcam_position_for(10.0, 10.0, (1920.0, 1080.0), (1920.0, 1080.0), 0.0),
            (0.0, 0.0)
        );
    }

    #[test]
    fn crop_edges_move_alone_and_keep_a_minimum() {
        let full = Crop::default();
        let left = dragged_crop(full, CropGrip::Left, 0.2);
        assert!((left.x - 0.2).abs() < 1e-9 && (left.width - 0.8).abs() < 1e-9);
        let right = dragged_crop(full, CropGrip::Right, 0.7);
        assert!((right.x).abs() < 1e-9 && (right.width - 0.7).abs() < 1e-9);
        let squeezed = dragged_crop(full, CropGrip::Bottom, -1.0);
        assert!((squeezed.height - SMALLEST_CROP_DRAG).abs() < 1e-9);
        let past = dragged_crop(right, CropGrip::Left, 0.95);
        assert!(past.width >= SMALLEST_CROP_DRAG - 1e-9);
    }

    fn annotation(id: &str, start: f64, end: f64) -> reel_core::timeline::Annotation {
        reel_core::timeline::Annotation {
            id: id.into(),
            start: TimelineTime::from_ms(start),
            end: TimelineTime::from_ms(end),
            content: reel_core::timeline::AnnotationContent::Blur {
                strength: 20.0,
                fill: None,
            },
            position: Point { x: 0.0, y: 0.0 },
            width: 10.0,
            height: 10.0,
            track: 0,
            layer: 0,
        }
    }

    #[test]
    fn tab_cycles_through_whats_under_the_playhead() {
        let list = [
            annotation("a", 0.0, 1000.0),
            annotation("b", 0.0, 1000.0),
            annotation("c", 5000.0, 6000.0),
        ];
        let now = TimelineTime::from_ms(500.0);
        let a = ItemId::from("a");
        let b = ItemId::from("b");
        assert_eq!(cycle_annotation(&list, now, None, false), Some(a.clone()));
        assert_eq!(
            cycle_annotation(&list, now, Some(&a), false),
            Some(b.clone())
        );
        assert_eq!(
            cycle_annotation(&list, now, Some(&b), false),
            Some(a.clone()),
            "it wraps around"
        );
        assert_eq!(cycle_annotation(&list, now, Some(&a), true), Some(b));
        assert_eq!(
            cycle_annotation(&list, TimelineTime::from_ms(3000.0), None, false),
            None
        );
    }
}
