//! Edits to the timeline, as commands, with the overlap rules in §14 and §15.
//!
//! Each edit either succeeds and leaves the timeline valid, or fails and changes nothing. Dragging
//! sends edits with the pointer's position, and most clamp it to something legal rather than fail.

use crate::time::{Micros, SourceTime, TimelineTime};
use crate::timeline::{
    Annotation, AnnotationContent, AudioItem, CLIP_SPEEDS, Caption, Clip, ItemId, MIN_ITEM, Point,
    Timeline, Zoom, ZoomMode,
};

/// A new zoom lasts this long, and needs this much room (§10).
pub const NEW_ZOOM: Micros = Micros(1_000_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Cuts the clip at this time into two, which show the same footage as before.
    SplitClip {
        at: TimelineTime,
    },
    /// Drags a clip's edge in or back out.
    TrimClip {
        id: ItemId,
        edge: Edge,
        to: TimelineTime,
    },
    /// Drags a clip along the timeline. It stops at a neighbour, or jumps past one once its centre
    /// has crossed the neighbour's, and it can go past the end to grow the timeline (§14).
    MoveClip {
        id: ItemId,
        to: TimelineTime,
    },
    /// Puts a clip at this place in the clip order, closing up the clips it passes.
    ReorderClip {
        id: ItemId,
        to_index: usize,
    },
    SetClipSpeed {
        id: ItemId,
        speed: f64,
    },
    SetClipMuted {
        id: ItemId,
        muted: bool,
    },
    SetClipSourceAudio {
        id: ItemId,
        show: bool,
    },
    /// Leaves a gap, which playback skips.
    DeleteClip {
        id: ItemId,
    },

    AddZoom {
        start: TimelineTime,
        depth: u8,
        focus: Point,
        mode: ZoomMode,
    },
    MoveZoom {
        id: ItemId,
        to: TimelineTime,
    },
    ResizeZoom {
        id: ItemId,
        edge: Edge,
        to: TimelineTime,
    },
    UpdateZoom {
        id: ItemId,
        depth: Option<u8>,
        focus: Option<Point>,
        mode: Option<ZoomMode>,
    },
    DeleteZoom {
        id: ItemId,
    },
    DeleteAllZooms,

    AddAnnotation {
        annotation: Annotation,
    },
    MoveAnnotation {
        id: ItemId,
        to: TimelineTime,
        track: Option<u32>,
    },
    ResizeAnnotation {
        id: ItemId,
        edge: Edge,
        to: TimelineTime,
    },
    UpdateAnnotation {
        id: ItemId,
        content: AnnotationContent,
    },
    /// Moves or resizes it on the preview, in percentages of the recording area.
    PlaceAnnotation {
        id: ItemId,
        position: Point,
        width: f64,
        height: f64,
    },
    DeleteAnnotation {
        id: ItemId,
    },

    AddAudio {
        item: AudioItem,
    },
    MoveAudio {
        id: ItemId,
        to: TimelineTime,
        track: Option<u32>,
    },
    /// Audio always plays from the start of its file, so the left edge moves the item rather than
    /// cutting into the sound (§16).
    ResizeAudio {
        id: ItemId,
        edge: Edge,
        to: TimelineTime,
    },
    SetAudioVolume {
        id: ItemId,
        volume: f64,
    },
    SetAudioBoost {
        id: ItemId,
        boost: bool,
    },
    DeleteAudio {
        id: ItemId,
    },

    AddCaption {
        start: TimelineTime,
        end: TimelineTime,
        text: String,
    },
    /// Rejected if it would overlap another caption (§14).
    MoveCaption {
        id: ItemId,
        to: TimelineTime,
    },
    ResizeCaption {
        id: ItemId,
        edge: Edge,
        to: TimelineTime,
    },
    SetCaptionText {
        id: ItemId,
        text: String,
    },
    DeleteCaption {
        id: ItemId,
    },
    /// Replaces them all, as generating captions does.
    SetCaptions {
        captions: Vec<Caption>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    NotFound,
    /// There's no clip at this time.
    NoClipHere,
    /// The result would be shorter than items are allowed to be.
    TooShort,
    /// It would overlap something it isn't allowed to.
    WouldOverlap,
    /// It's not one of the allowed speeds.
    BadSpeed,
    /// There's nowhere legal for it to go.
    NoRoom,
    /// A project always keeps at least one clip.
    LastClip,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "That item no longer exists.",
            Self::NoClipHere => "There's no clip here.",
            Self::TooShort => "That would make it too short.",
            Self::WouldOverlap => "That would overlap something else.",
            Self::BadSpeed => "That speed isn't available.",
            Self::NoRoom => "There isn't room for that here.",
            Self::LastClip => "A project needs at least one clip.",
        })
    }
}

impl std::error::Error for EditError {}

/// What an edit created, for selecting it afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub created: Option<ItemId>,
}

type EditResult = Result<Outcome, EditError>;

fn done() -> EditResult {
    Ok(Outcome::default())
}

impl Timeline {
    /// Applies an edit. On failure, the timeline is unchanged.
    pub fn apply(&mut self, edit: &Edit) -> EditResult {
        let mut next = self.clone();
        let outcome = next.apply_inner(edit)?;
        next.sort_items();
        *self = next;
        Ok(outcome)
    }

    fn apply_inner(&mut self, edit: &Edit) -> EditResult {
        match edit {
            Edit::SplitClip { at } => self.split_clip(*at),
            Edit::TrimClip { id, edge, to } => self.trim_clip(id, *edge, *to),
            Edit::MoveClip { id, to } => self.move_clip(id, *to),
            Edit::ReorderClip { id, to_index } => self.reorder_clip(id, *to_index),
            Edit::SetClipSpeed { id, speed } => self.set_clip_speed(id, *speed),
            Edit::SetClipMuted { id, muted } => {
                self.clip_mut(id)?.muted = *muted;
                done()
            }
            Edit::SetClipSourceAudio { id, show } => {
                self.clip_mut(id)?.show_source_audio = *show;
                done()
            }
            Edit::DeleteClip { id } => {
                let index = self.clip_index(id)?;
                if self.clips.len() == 1 {
                    return Err(EditError::LastClip);
                }
                self.clips.remove(index);
                done()
            }

            Edit::AddZoom {
                start,
                depth,
                focus,
                mode,
            } => self.add_zoom(*start, *depth, *focus, *mode),
            Edit::MoveZoom { id, to } => self.move_zoom(id, *to),
            Edit::ResizeZoom { id, edge, to } => self.resize_zoom(id, *edge, *to),
            Edit::UpdateZoom {
                id,
                depth,
                focus,
                mode,
            } => {
                let zoom = self.zoom_mut(id)?;
                if let Some(depth) = depth {
                    zoom.depth = (*depth).min(5);
                }
                if let Some(focus) = focus {
                    zoom.focus = Point {
                        x: focus.x.clamp(0.0, 1.0),
                        y: focus.y.clamp(0.0, 1.0),
                    };
                }
                if let Some(mode) = mode {
                    zoom.mode = *mode;
                }
                done()
            }
            Edit::DeleteZoom { id } => {
                let index = self
                    .zooms
                    .iter()
                    .position(|item| item.id == *id)
                    .ok_or(EditError::NotFound)?;
                self.zooms.remove(index);
                done()
            }
            Edit::DeleteAllZooms => {
                self.zooms.clear();
                done()
            }

            Edit::AddAnnotation { annotation } => {
                let mut annotation = annotation.clone();
                annotation.id = self.new_id("annotation");
                annotation.start = annotation.start.max(TimelineTime::ZERO);
                annotation.end = annotation.end.max(annotation.start + MIN_ITEM);
                let id = annotation.id.clone();
                self.annotations.push(annotation);
                Ok(Outcome { created: Some(id) })
            }
            Edit::MoveAnnotation { id, to, track } => {
                let item = self.annotation_mut(id)?;
                let length = item.end - item.start;
                item.start = (*to).max(TimelineTime::ZERO);
                item.end = item.start + length;
                if let Some(track) = track {
                    item.track = *track;
                }
                done()
            }
            Edit::ResizeAnnotation { id, edge, to } => {
                let item = self.annotation_mut(id)?;
                match edge {
                    Edge::Start => {
                        item.start = (*to).max(TimelineTime::ZERO).min(item.end - MIN_ITEM)
                    }
                    Edge::End => item.end = (*to).max(item.start + MIN_ITEM),
                }
                done()
            }
            Edit::UpdateAnnotation { id, content } => {
                self.annotation_mut(id)?.content = content.clone();
                done()
            }
            Edit::PlaceAnnotation {
                id,
                position,
                width,
                height,
            } => {
                let item = self.annotation_mut(id)?;
                item.position = *position;
                item.width = *width;
                item.height = *height;
                done()
            }
            Edit::DeleteAnnotation { id } => {
                let index = self
                    .annotations
                    .iter()
                    .position(|item| item.id == *id)
                    .ok_or(EditError::NotFound)?;
                self.annotations.remove(index);
                done()
            }

            Edit::AddAudio { item } => self.add_audio(item),
            Edit::MoveAudio { id, to, track } => self.move_audio(id, *to, *track),
            Edit::ResizeAudio { id, edge, to } => self.resize_audio(id, *edge, *to),
            Edit::SetAudioVolume { id, volume } => {
                self.audio_mut(id)?.volume = volume.clamp(0.0, 1.0);
                done()
            }
            Edit::SetAudioBoost { id, boost } => {
                self.audio_mut(id)?.boost = *boost;
                done()
            }
            Edit::DeleteAudio { id } => {
                let index = self
                    .audio
                    .iter()
                    .position(|item| item.id == *id)
                    .ok_or(EditError::NotFound)?;
                self.audio.remove(index);
                done()
            }

            Edit::AddCaption { start, end, text } => self.add_caption(*start, *end, text),
            Edit::MoveCaption { id, to } => self.move_caption(id, *to),
            Edit::ResizeCaption { id, edge, to } => self.resize_caption(id, *edge, *to),
            Edit::SetCaptionText { id, text } => {
                let caption = self.caption_mut(id)?;
                caption.text.clone_from(text);
                // The word timings no longer describe this text.
                caption.words.clear();
                done()
            }
            Edit::DeleteCaption { id } => {
                let index = self
                    .captions
                    .iter()
                    .position(|item| item.id == *id)
                    .ok_or(EditError::NotFound)?;
                self.captions.remove(index);
                done()
            }
            Edit::SetCaptions { captions } => {
                self.captions = captions
                    .iter()
                    .filter(|cue| !cue.text.trim().is_empty())
                    .cloned()
                    .collect();
                self.captions.sort_by_key(|cue| cue.start);
                done()
            }
        }
    }

    fn sort_items(&mut self) {
        self.clips.sort_by_key(|item| item.start);
        self.zooms.sort_by_key(|item| item.start);
        self.annotations.sort_by_key(|item| item.start);
        self.audio.sort_by_key(|item| (item.track, item.start));
        self.captions.sort_by_key(|item| item.start);
    }

    fn clip_index(&self, id: &ItemId) -> Result<usize, EditError> {
        self.clips
            .iter()
            .position(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    fn clip_mut(&mut self, id: &ItemId) -> Result<&mut Clip, EditError> {
        self.clips
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    fn zoom_mut(&mut self, id: &ItemId) -> Result<&mut Zoom, EditError> {
        self.zooms
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    fn annotation_mut(&mut self, id: &ItemId) -> Result<&mut Annotation, EditError> {
        self.annotations
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    fn audio_mut(&mut self, id: &ItemId) -> Result<&mut AudioItem, EditError> {
        self.audio
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    fn caption_mut(&mut self, id: &ItemId) -> Result<&mut Caption, EditError> {
        self.captions
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)
    }

    // ---- Clips ----

    fn split_clip(&mut self, at: TimelineTime) -> EditResult {
        let index = self
            .clips
            .iter()
            .position(|clip| clip.contains(at))
            .ok_or(EditError::NoClipHere)?;
        let left_length = at - self.clips[index].start;
        let right_length = self.clips[index].duration - left_length;
        if left_length < MIN_ITEM || right_length < MIN_ITEM {
            return Err(EditError::TooShort);
        }
        let id = self.new_id("clip");
        let mut right = self.clips[index].clone();
        right.id = id.clone();
        right.source_start = self.clips[index].source_at(at);
        right.start = at;
        right.duration = right_length;
        self.clips[index].duration = left_length;
        self.clips.insert(index + 1, right);
        Ok(Outcome { created: Some(id) })
    }

    fn trim_clip(&mut self, id: &ItemId, edge: Edge, to: TimelineTime) -> EditResult {
        let index = self.clip_index(id)?;
        let clip = self.clips[index].clone();
        let before = index.checked_sub(1).map(|i| self.clips[i].end());
        let after = self.clips.get(index + 1).map(|next| next.start);
        match edge {
            Edge::Start => {
                // Stop where the recording begins, or at the previous clip.
                let recording_start =
                    clip.start - (clip.source_start - SourceTime::ZERO).scaled(1.0 / clip.speed);
                let lowest = recording_start
                    .max(before.unwrap_or(TimelineTime::ZERO))
                    .max(TimelineTime::ZERO);
                let highest = clip.end() - MIN_ITEM;
                let start = to.min(highest).max(lowest);
                let source_start = clip.source_at(start).max(SourceTime::ZERO);
                let trimmed = &mut self.clips[index];
                trimmed.start = start;
                trimmed.duration = clip.end() - start;
                trimmed.source_start = source_start;
                if start > clip.start {
                    self.clear_span(clip.start, start);
                }
            }
            Edge::End => {
                // Stop where the recording ends, or at the next clip.
                let available = (SourceTime(self.source_duration.0) - clip.source_start)
                    .scaled(1.0 / clip.speed);
                let mut highest = clip.start + available.max(MIN_ITEM);
                if let Some(next_start) = after {
                    highest = highest.min(next_start);
                }
                let end = to
                    .max(clip.start + MIN_ITEM)
                    .min(highest)
                    .max(clip.start + MIN_ITEM);
                self.clips[index].duration = end - clip.start;
                if end < clip.end() {
                    self.clear_span(end, clip.end());
                }
            }
        }
        done()
    }

    /// Makes room after a trim. Items entirely inside the removed part go. Recordly also deletes
    /// anything partly inside it, which loses work (BUILD_PLAN 6.6), so those are shortened instead.
    fn clear_span(&mut self, from: TimelineTime, to: TimelineTime) {
        fn trim(
            start: &mut TimelineTime,
            end: &mut TimelineTime,
            from: TimelineTime,
            to: TimelineTime,
        ) -> bool {
            let overlaps = *start < to && *end > from;
            if !overlaps {
                return true;
            }
            if *start >= from && *end <= to {
                return false;
            }
            if *start < from && *end > to {
                // It spans the whole removed part, so it still has footage on both sides.
                return true;
            }
            if *start < from {
                *end = from;
            } else {
                *start = to;
            }
            *end - *start >= MIN_ITEM
        }
        self.zooms
            .retain_mut(|item| trim(&mut item.start, &mut item.end, from, to));
        self.annotations
            .retain_mut(|item| trim(&mut item.start, &mut item.end, from, to));
        self.audio
            .retain_mut(|item| trim(&mut item.start, &mut item.end, from, to));
    }

    fn move_clip(&mut self, id: &ItemId, to: TimelineTime) -> EditResult {
        let index = self.clip_index(id)?;
        let clip = self.clips[index].clone();
        let moving_right = to > clip.start;
        let others: Vec<(TimelineTime, TimelineTime)> = self
            .clips
            .iter()
            .filter(|other| other.id != *id)
            .map(|other| (other.start, other.end()))
            .collect();
        let start = place_clip(
            &others,
            clip.duration,
            to.max(TimelineTime::ZERO),
            moving_right,
        )
        .ok_or(EditError::NoRoom)?;
        let delta = start - clip.start;
        self.clips[index].start = start;

        // Zooms inside the clip go with it (§15). Annotations and audio stay where they are, and
        // aren't deleted as they are in Recordly: they can simply be moved back.
        let (mut moving, staying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.zooms)
            .into_iter()
            .partition(|zoom| zoom.start >= clip.start && zoom.start < clip.end());
        for zoom in &mut moving {
            zoom.start += delta;
            zoom.end += delta;
        }
        let overlap = moving.iter().any(|zoom| {
            staying
                .iter()
                .any(|other| zoom.start < other.end && other.start < zoom.end)
        });
        if overlap {
            return Err(EditError::WouldOverlap);
        }
        self.zooms = staying;
        self.zooms.extend(moving);
        done()
    }

    fn reorder_clip(&mut self, id: &ItemId, to_index: usize) -> EditResult {
        let from = self.clip_index(id)?;
        let to = to_index.min(self.clips.len() - 1);
        if from == to {
            return done();
        }
        let first = from.min(to);
        let last = from.max(to);
        let originals: Vec<Clip> = self.clips.clone();

        let moved = self.clips.remove(from);
        self.clips.insert(to, moved);

        // Close up the clips this passes. They take up no more room than before, because gaps only
        // shrink, so nothing after them needs to move.
        let mut cursor = originals[first].start;
        for clip in &mut self.clips[first..=last] {
            clip.start = cursor;
            cursor += clip.duration;
        }

        // Each zoom goes with the clip it starts inside.
        let mut zooms = std::mem::take(&mut self.zooms);
        for zoom in &mut zooms {
            let owner = originals[first..=last]
                .iter()
                .find(|clip| zoom.start >= clip.start && zoom.start < clip.end());
            if let Some(owner) = owner {
                let new_start = self
                    .clips
                    .iter()
                    .find(|clip| clip.id == owner.id)
                    .map_or(owner.start, |clip| clip.start);
                let delta = new_start - owner.start;
                zoom.start += delta;
                zoom.end += delta;
            }
        }
        zooms.sort_by_key(|zoom| zoom.start);
        // A zoom that ran past its clip could now hit the next one, so shorten it to fit.
        for index in 1..zooms.len() {
            let limit = zooms[index].start;
            if zooms[index - 1].end > limit {
                zooms[index - 1].end = limit;
                if zooms[index - 1].end - zooms[index - 1].start < MIN_ITEM {
                    return Err(EditError::WouldOverlap);
                }
            }
        }
        self.zooms = zooms;
        done()
    }

    fn set_clip_speed(&mut self, id: &ItemId, speed: f64) -> EditResult {
        if !CLIP_SPEEDS.contains(&speed) {
            return Err(EditError::BadSpeed);
        }
        let index = self.clip_index(id)?;
        let clip = self.clips[index].clone();
        let source_length = clip.source_end() - clip.source_start;
        let duration = source_length.scaled(1.0 / speed);
        if duration < MIN_ITEM {
            return Err(EditError::TooShort);
        }
        // Later clips never shift (§15), so a longer clip has to fit before the next one.
        if let Some(next) = self.clips.get(index + 1)
            && clip.start + duration > next.start
        {
            return Err(EditError::WouldOverlap);
        }
        let ratio = duration.0 as f64 / clip.duration.0 as f64;

        // Zooms that start inside the clip keep their place in the footage (§15).
        let zooms = &mut self.zooms;
        let inside: Vec<usize> = zooms
            .iter()
            .enumerate()
            .filter(|(_, zoom)| zoom.start >= clip.start && zoom.start < clip.end())
            .map(|(i, _)| i)
            .collect();
        for &i in &inside {
            let zoom = &mut zooms[i];
            zoom.start = clip.start + (zoom.start - clip.start).scaled(ratio);
            zoom.end = clip.start + (zoom.end - clip.start).scaled(ratio);
            if zoom.end - zoom.start < MIN_ITEM {
                return Err(EditError::TooShort);
            }
        }
        zooms.sort_by_key(|zoom| zoom.start);
        if zooms.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return Err(EditError::WouldOverlap);
        }

        let changed = &mut self.clips[index];
        changed.speed = speed;
        changed.duration = duration;
        done()
    }

    // ---- Zooms ----

    /// The gap a zoom can sit in, between its neighbours.
    fn zoom_gap(
        &self,
        id: Option<&ItemId>,
        around: TimelineTime,
    ) -> (TimelineTime, Option<TimelineTime>) {
        let mut low = TimelineTime::ZERO;
        let mut high = None;
        for zoom in self.zooms.iter().filter(|zoom| Some(&zoom.id) != id) {
            if zoom.start <= around {
                low = low.max(zoom.end);
            } else if high.is_none_or(|limit| zoom.start < limit) {
                high = Some(zoom.start);
            }
        }
        (low, high)
    }

    fn add_zoom(
        &mut self,
        start: TimelineTime,
        depth: u8,
        focus: Point,
        mode: ZoomMode,
    ) -> EditResult {
        // A zoom must start inside a clip, must not overlap another, and needs a full second before
        // the next zoom or the clip's end (§10).
        let clip = self.clip_at(start).ok_or(EditError::NoClipHere)?;
        let clip_end = clip.end();
        let end = start + NEW_ZOOM;
        if end > clip_end {
            return Err(EditError::NoRoom);
        }
        if self
            .zooms
            .iter()
            .any(|zoom| start < zoom.end && zoom.start < end)
        {
            return Err(EditError::WouldOverlap);
        }
        let id = self.new_id("zoom");
        self.zooms.push(Zoom {
            id: id.clone(),
            start,
            end,
            depth: depth.min(5),
            focus,
            mode,
        });
        Ok(Outcome { created: Some(id) })
    }

    fn move_zoom(&mut self, id: &ItemId, to: TimelineTime) -> EditResult {
        let zoom = self
            .zooms
            .iter()
            .find(|zoom| zoom.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let length = zoom.end - zoom.start;
        let (low, high) = self.zoom_gap(Some(id), zoom.start);
        let mut start = to.max(low);
        if let Some(high) = high {
            start = start.min(high - length);
        }
        let start = start.max(low);
        let moved = self.zoom_mut(id)?;
        moved.start = start;
        moved.end = start + length;
        done()
    }

    fn resize_zoom(&mut self, id: &ItemId, edge: Edge, to: TimelineTime) -> EditResult {
        let zoom = self
            .zooms
            .iter()
            .find(|zoom| zoom.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let (low, high) = self.zoom_gap(Some(id), zoom.start);
        let resized = self.zoom_mut(id)?;
        match edge {
            Edge::Start => resized.start = to.max(low).min(zoom.end - MIN_ITEM),
            Edge::End => {
                let highest = high.unwrap_or(TimelineTime(i64::MAX));
                resized.end = to.min(highest).max(zoom.start + MIN_ITEM);
            }
        }
        done()
    }

    // ---- Audio ----

    fn add_audio(&mut self, item: &AudioItem) -> EditResult {
        let mut item = item.clone();
        item.id = self.new_id("audio");
        // A new item lands on a new track, so it never overlaps (§16).
        item.track = self
            .audio
            .iter()
            .map(|other| other.track + 1)
            .max()
            .unwrap_or(0);
        item.start = item.start.max(TimelineTime::ZERO);
        let mut length = item.end - item.start;
        if item.file_duration > Micros::ZERO {
            length = length.min(item.file_duration);
        }
        if length < MIN_ITEM {
            return Err(EditError::TooShort);
        }
        item.end = item.start + length;
        let id = item.id.clone();
        self.audio.push(item);
        Ok(Outcome { created: Some(id) })
    }

    /// The free space on a track around a time.
    fn audio_gap(
        &self,
        id: &ItemId,
        track: u32,
        around: TimelineTime,
    ) -> (TimelineTime, Option<TimelineTime>) {
        let mut low = TimelineTime::ZERO;
        let mut high = None;
        for item in self
            .audio
            .iter()
            .filter(|item| item.track == track && item.id != *id)
        {
            if item.start <= around {
                low = low.max(item.end);
            } else if high.is_none_or(|limit| item.start < limit) {
                high = Some(item.start);
            }
        }
        (low, high)
    }

    fn move_audio(&mut self, id: &ItemId, to: TimelineTime, track: Option<u32>) -> EditResult {
        let item = self
            .audio
            .iter()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let track = track.unwrap_or(item.track);
        let length = item.end - item.start;
        // On its own track it keeps within its current gap. On another, it lands where it's dropped.
        let around = if track == item.track { item.start } else { to };
        let (low, high) = self.audio_gap(id, track, around);
        let mut start = to.max(low);
        if let Some(high) = high {
            if high - low < length {
                return Err(EditError::NoRoom);
            }
            start = start.min(high - length);
        }
        let moved = self.audio_mut(id)?;
        moved.start = start;
        moved.end = start + length;
        moved.track = track;
        done()
    }

    fn resize_audio(&mut self, id: &ItemId, edge: Edge, to: TimelineTime) -> EditResult {
        let item = self
            .audio
            .iter()
            .find(|item| item.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let (low, high) = self.audio_gap(id, item.track, item.start);
        let longest = if item.file_duration > Micros::ZERO {
            item.file_duration
        } else {
            Micros(i64::MAX / 4)
        };
        let resized = self.audio_mut(id)?;
        match edge {
            Edge::Start => {
                let lowest = low.max(item.end - longest);
                resized.start = to.max(lowest).min(item.end - MIN_ITEM);
            }
            Edge::End => {
                let highest =
                    high.map_or(item.start + longest, |high| high.min(item.start + longest));
                resized.end = to.min(highest).max(item.start + MIN_ITEM);
            }
        }
        done()
    }

    // ---- Captions ----

    fn caption_overlaps(
        &self,
        id: Option<&ItemId>,
        start: TimelineTime,
        end: TimelineTime,
    ) -> bool {
        self.captions
            .iter()
            .filter(|cue| Some(&cue.id) != id)
            .any(|cue| start < cue.end && cue.start < end)
    }

    fn add_caption(&mut self, start: TimelineTime, end: TimelineTime, text: &str) -> EditResult {
        if end - start < MIN_ITEM || start < TimelineTime::ZERO {
            return Err(EditError::TooShort);
        }
        if self.caption_overlaps(None, start, end) {
            return Err(EditError::WouldOverlap);
        }
        let id = self.new_id("caption");
        self.captions.push(Caption {
            id: id.clone(),
            start,
            end,
            text: text.to_string(),
            words: vec![],
        });
        Ok(Outcome { created: Some(id) })
    }

    fn move_caption(&mut self, id: &ItemId, to: TimelineTime) -> EditResult {
        let cue = self
            .captions
            .iter()
            .find(|cue| cue.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let length = cue.end - cue.start;
        let start = to.max(TimelineTime::ZERO);
        if self.caption_overlaps(Some(id), start, start + length) {
            return Err(EditError::WouldOverlap);
        }
        let delta = start - cue.start;
        let moved = self.caption_mut(id)?;
        moved.start = start;
        moved.end = start + length;
        for word in &mut moved.words {
            word.start += delta;
            word.end += delta;
        }
        done()
    }

    fn resize_caption(&mut self, id: &ItemId, edge: Edge, to: TimelineTime) -> EditResult {
        let cue = self
            .captions
            .iter()
            .find(|cue| cue.id == *id)
            .ok_or(EditError::NotFound)?
            .clone();
        let (start, end) = match edge {
            Edge::Start => (to.max(TimelineTime::ZERO), cue.end),
            Edge::End => (cue.start, to),
        };
        if end - start < MIN_ITEM {
            return Err(EditError::TooShort);
        }
        if self.caption_overlaps(Some(id), start, end) {
            return Err(EditError::WouldOverlap);
        }
        let resized = self.caption_mut(id)?;
        resized.start = start;
        resized.end = end;
        // Stretching a caption by hand makes its word timings meaningless.
        resized.words.clear();
        done()
    }

    /// Checks everything the edits are meant to guarantee.
    ///
    /// # Errors
    /// Describes the first problem found.
    pub fn check(&self) -> Result<(), String> {
        for clip in &self.clips {
            if clip.duration < MIN_ITEM {
                return Err(format!("clip {} is too short", clip.id.0));
            }
            if clip.start < TimelineTime::ZERO || clip.source_start < SourceTime::ZERO {
                return Err(format!("clip {} starts before zero", clip.id.0));
            }
        }
        if self
            .clips
            .windows(2)
            .any(|pair| pair[0].end() > pair[1].start)
        {
            return Err("clips overlap or are out of order".to_string());
        }
        for zoom in &self.zooms {
            if zoom.end - zoom.start < MIN_ITEM {
                return Err(format!("zoom {} is too short", zoom.id.0));
            }
        }
        if self
            .zooms
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start)
        {
            return Err("zooms overlap or are out of order".to_string());
        }
        if self
            .captions
            .windows(2)
            .any(|pair| pair[0].end > pair[1].start)
        {
            return Err("captions overlap or are out of order".to_string());
        }
        if self
            .audio
            .windows(2)
            .any(|pair| pair[0].track == pair[1].track && pair[0].end > pair[1].start)
        {
            return Err("audio overlaps on a track".to_string());
        }
        for item in &self.annotations {
            if item.end - item.start < MIN_ITEM {
                return Err(format!("annotation {} is too short", item.id.0));
            }
        }
        for item in &self.audio {
            if item.end - item.start < MIN_ITEM {
                return Err(format!("audio {} is too short", item.id.0));
            }
        }
        Ok(())
    }
}

/// Finds where a dragged clip lands. It stops against a neighbour, or jumps past one once its centre
/// has crossed the neighbour's centre (§14). `None` means no legal place was found.
fn place_clip(
    others: &[(TimelineTime, TimelineTime)],
    length: Micros,
    to: TimelineTime,
    moving_right: bool,
) -> Option<TimelineTime> {
    let center = |start: TimelineTime, end: TimelineTime| (start.0 + end.0) / 2;
    let mut candidate = to;
    for _ in 0..=others.len() * 2 + 1 {
        if candidate < TimelineTime::ZERO {
            return None;
        }
        let end = candidate + length;
        let hit = others
            .iter()
            .copied()
            .find(|(start, stop)| candidate < *stop && *start < end);
        let Some((start, stop)) = hit else {
            return Some(candidate);
        };
        let crossed = if moving_right {
            center(candidate, end) > center(start, stop)
        } else {
            center(candidate, end) < center(start, stop)
        };
        candidate = match (moving_right, crossed) {
            (true, true) | (false, false) => stop,
            (true, false) | (false, true) => start - length,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ms(value: f64) -> TimelineTime {
        TimelineTime::from_ms(value)
    }

    fn timeline() -> Timeline {
        Timeline::for_recording(Micros::from_secs(30))
    }

    fn apply(timeline: &mut Timeline, edit: Edit) -> Outcome {
        let outcome = timeline.apply(&edit).unwrap();
        timeline.check().unwrap();
        outcome
    }

    #[test]
    fn splitting_keeps_every_frame_where_it_was_at_any_speed() {
        // Recordly misaligns the right half at speeds other than 1× (§58).
        for speed in CLIP_SPEEDS {
            let mut t = timeline();
            t.apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed,
            })
            .unwrap();
            let before = t.clone();
            let at = TimelineTime(t.clips[0].duration.0 / 3);
            t.apply(&Edit::SplitClip { at }).unwrap();
            t.check().unwrap();
            for step in 0..200 {
                let time = TimelineTime(before.clips[0].duration.0 * step / 200);
                let (a, b) = (before.to_source(time), t.to_source(time));
                let difference = (a.unwrap().0 - b.unwrap().0).abs();
                assert!(
                    difference as f64 <= speed.max(1.0) + 1.0,
                    "off by {difference} µs at speed {speed}"
                );
            }
            assert_eq!(
                t.clips[0].end(),
                t.clips[1].start,
                "no gap or overlap at speed {speed}"
            );
        }
    }

    #[test]
    fn a_split_too_near_an_edge_is_refused() {
        let mut t = timeline();
        assert_eq!(
            t.apply(&Edit::SplitClip { at: ms(50.0) }),
            Err(EditError::TooShort)
        );
    }

    #[test]
    fn a_moved_clip_keeps_showing_the_same_footage() {
        // Recordly's clips have no source position, so moving one changes what it shows (§15).
        let mut t = timeline();
        t.apply(&Edit::TrimClip {
            id: "clip-1".into(),
            edge: Edge::End,
            to: ms(5000.0),
        })
        .unwrap();
        let source_at_start = t.clips[0].source_start;
        apply(
            &mut t,
            Edit::MoveClip {
                id: "clip-1".into(),
                to: ms(10_000.0),
            },
        );
        assert_eq!(t.clips[0].start, ms(10_000.0));
        assert_eq!(t.to_source(ms(10_000.0)), Some(source_at_start));
    }

    #[test]
    fn clips_can_really_be_reordered() {
        let mut t = timeline();
        apply(&mut t, Edit::SplitClip { at: ms(10_000.0) });
        apply(&mut t, Edit::SplitClip { at: ms(20_000.0) });
        let ids: Vec<_> = t.clips.iter().map(|clip| clip.id.clone()).collect();
        let last_source = t.clips[2].source_start;
        apply(
            &mut t,
            Edit::ReorderClip {
                id: ids[2].clone(),
                to_index: 0,
            },
        );
        assert_eq!(t.clips[0].id, ids[2]);
        assert_eq!(t.clips[0].source_start, last_source);
        assert_eq!(t.clips[0].start, TimelineTime::ZERO);
        assert_eq!(t.clips[1].id, ids[0]);
        assert_eq!(t.clips[1].start, ms(10_000.0));
    }

    #[test]
    fn a_zoom_goes_with_its_clip_when_reordering() {
        let mut t = timeline();
        apply(&mut t, Edit::SplitClip { at: ms(10_000.0) });
        let second = t.clips[1].id.clone();
        apply(
            &mut t,
            Edit::AddZoom {
                start: ms(12_000.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            },
        );
        apply(
            &mut t,
            Edit::ReorderClip {
                id: second,
                to_index: 0,
            },
        );
        assert_eq!(t.zooms[0].start, ms(2000.0));
    }

    #[test]
    fn a_dragged_clip_stops_at_a_neighbour_until_its_centre_crosses() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::End,
                to: ms(4000.0),
            },
        );
        apply(&mut t, Edit::SplitClip { at: ms(2000.0) });
        let right = t.clips[1].id.clone();
        // Move the right one away, leaving room on either side of it.
        apply(
            &mut t,
            Edit::MoveClip {
                id: right.clone(),
                to: ms(10_000.0),
            },
        );
        let left = t.clips[0].id.clone();

        // Dragged to just past the neighbour's start, it stops against it.
        apply(
            &mut t,
            Edit::MoveClip {
                id: left.clone(),
                to: ms(8500.0),
            },
        );
        assert_eq!(
            t.clips.iter().find(|c| c.id == left).unwrap().start,
            ms(8000.0)
        );

        // With its centre past the neighbour's, it jumps to the far side.
        apply(
            &mut t,
            Edit::MoveClip {
                id: left.clone(),
                to: ms(10_500.0),
            },
        );
        assert_eq!(
            t.clips.iter().find(|c| c.id == left).unwrap().start,
            ms(12_000.0)
        );
    }

    #[test]
    fn trimming_shortens_items_rather_than_deleting_them() {
        // Recordly deletes anything that overlaps the removed part, even partly (§15).
        let mut t = timeline();
        apply(
            &mut t,
            Edit::AddZoom {
                start: ms(4500.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            },
        );
        apply(
            &mut t,
            Edit::ResizeZoom {
                id: "zoom-1".into(),
                edge: Edge::End,
                to: ms(6000.0),
            },
        );
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::End,
                to: ms(5000.0),
            },
        );
        assert_eq!(t.zooms.len(), 1);
        assert_eq!(t.zooms[0].end, ms(5000.0));
    }

    #[test]
    fn trimming_removes_what_is_entirely_in_the_removed_part() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::AddZoom {
                start: ms(20_000.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            },
        );
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::End,
                to: ms(5000.0),
            },
        );
        assert!(t.zooms.is_empty());
    }

    #[test]
    fn trimming_the_start_keeps_the_remaining_footage_in_place() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::Start,
                to: ms(3000.0),
            },
        );
        assert_eq!(t.clips[0].start, ms(3000.0));
        assert_eq!(t.to_source(ms(3000.0)), Some(SourceTime::from_ms(3000.0)));
        assert_eq!(t.clips[0].end(), ms(30_000.0));
    }

    #[test]
    fn a_trim_can_extend_back_out_but_not_past_the_recording() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::End,
                to: ms(5000.0),
            },
        );
        apply(
            &mut t,
            Edit::TrimClip {
                id: "clip-1".into(),
                edge: Edge::End,
                to: ms(99_000.0),
            },
        );
        assert_eq!(t.clips[0].end(), ms(30_000.0));
    }

    #[test]
    fn a_speed_change_that_would_hit_the_next_clip_is_blocked() {
        let mut t = timeline();
        apply(&mut t, Edit::SplitClip { at: ms(10_000.0) });
        let first = t.clips[0].id.clone();
        let before = t.clone();
        assert_eq!(
            t.apply(&Edit::SetClipSpeed {
                id: first,
                speed: 0.5
            }),
            Err(EditError::WouldOverlap)
        );
        assert_eq!(t, before, "a refused edit changes nothing");
    }

    #[test]
    fn a_speed_change_rescales_zooms_inside_the_clip_and_nothing_else() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::AddZoom {
                start: ms(10_000.0),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            },
        );
        apply(
            &mut t,
            Edit::AddCaption {
                start: ms(10_000.0),
                end: ms(12_000.0),
                text: "hi".to_string(),
            },
        );
        apply(
            &mut t,
            Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 2.0,
            },
        );
        assert_eq!(t.clips[0].duration, Micros::from_secs(15));
        assert_eq!(t.zooms[0].start, ms(5000.0));
        assert_eq!(t.zooms[0].end - t.zooms[0].start, Micros::from_ms(500.0));
        assert_eq!(t.captions[0].start, ms(10_000.0));
    }

    #[test]
    fn only_listed_speeds_are_allowed() {
        let mut t = timeline();
        assert_eq!(
            t.apply(&Edit::SetClipSpeed {
                id: "clip-1".into(),
                speed: 1.75
            }),
            Err(EditError::BadSpeed)
        );
    }

    #[test]
    fn deleting_a_clip_leaves_a_gap_and_later_clips_stay_put() {
        let mut t = timeline();
        apply(&mut t, Edit::SplitClip { at: ms(10_000.0) });
        let second_start = t.clips[1].start;
        let first = t.clips[0].id.clone();
        apply(&mut t, Edit::DeleteClip { id: first });
        assert_eq!(t.clips[0].start, second_start);
        assert_eq!(t.to_source(ms(5000.0)), None);
        let last = t.clips[0].id.clone();
        assert_eq!(
            t.apply(&Edit::DeleteClip { id: last }),
            Err(EditError::LastClip)
        );
    }

    #[test]
    fn a_new_zoom_needs_a_clip_and_a_full_second() {
        let mut t = timeline();
        let add = |start| Edit::AddZoom {
            start: ms(start),
            depth: 1,
            focus: Point::CENTER,
            mode: ZoomMode::Auto,
        };
        assert_eq!(t.apply(&add(31_000.0)), Err(EditError::NoClipHere));
        assert_eq!(t.apply(&add(29_500.0)), Err(EditError::NoRoom));
        apply(&mut t, add(5000.0));
        assert_eq!(t.apply(&add(5500.0)), Err(EditError::WouldOverlap));
    }

    #[test]
    fn a_dragged_zoom_stops_at_its_neighbours() {
        let mut t = timeline();
        let add = |start| Edit::AddZoom {
            start: ms(start),
            depth: 1,
            focus: Point::CENTER,
            mode: ZoomMode::Auto,
        };
        apply(&mut t, add(5000.0));
        apply(&mut t, add(10_000.0));
        apply(
            &mut t,
            Edit::MoveZoom {
                id: "zoom-1".into(),
                to: ms(20_000.0),
            },
        );
        assert_eq!(t.zooms[0].end, ms(10_000.0));
        apply(
            &mut t,
            Edit::MoveZoom {
                id: "zoom-2".into(),
                to: ms(0.0),
            },
        );
        assert_eq!(t.zooms[1].start, ms(10_000.0));
    }

    #[test]
    fn a_resized_zoom_stops_at_its_neighbours() {
        let mut t = timeline();
        let add = |start| Edit::AddZoom {
            start: ms(start),
            depth: 1,
            focus: Point::CENTER,
            mode: ZoomMode::Auto,
        };
        apply(&mut t, add(5000.0));
        apply(&mut t, add(10_000.0));
        apply(
            &mut t,
            Edit::ResizeZoom {
                id: "zoom-1".into(),
                edge: Edge::End,
                to: ms(25_000.0),
            },
        );
        assert_eq!(t.zooms[0].end, ms(10_000.0));
        apply(
            &mut t,
            Edit::ResizeZoom {
                id: "zoom-2".into(),
                edge: Edge::Start,
                to: ms(0.0),
            },
        );
        assert_eq!(t.zooms[1].start, ms(10_000.0));
    }

    #[test]
    fn annotations_may_overlap_and_change_rows() {
        let mut t = timeline();
        let annotation = Annotation {
            id: "".into(),
            start: ms(1000.0),
            end: ms(2000.0),
            content: AnnotationContent::Blur {
                strength: 20.0,
                fill: None,
            },
            position: Point { x: 50.0, y: 50.0 },
            width: 30.0,
            height: 20.0,
            track: 0,
            layer: 0,
        };
        apply(
            &mut t,
            Edit::AddAnnotation {
                annotation: annotation.clone(),
            },
        );
        apply(&mut t, Edit::AddAnnotation { annotation });
        assert_eq!(t.annotations.len(), 2);
        let id = t.annotations[1].id.clone();
        apply(
            &mut t,
            Edit::MoveAnnotation {
                id: id.clone(),
                to: ms(1500.0),
                track: Some(2),
            },
        );
        assert_eq!(t.annotations.iter().find(|a| a.id == id).unwrap().track, 2);
    }

    fn audio(start: f64, end: f64, file_ms: f64) -> AudioItem {
        AudioItem {
            id: "".into(),
            start: ms(start),
            end: ms(end),
            file: crate::project::MediaRef::absolute("/a.wav"),
            file_duration: Micros::from_ms(file_ms),
            volume: 1.0,
            boost: false,
            track: 0,
        }
    }

    #[test]
    fn audio_never_runs_longer_than_its_file() {
        let mut t = timeline();
        let outcome = apply(
            &mut t,
            Edit::AddAudio {
                item: audio(0.0, 10_000.0, 3000.0),
            },
        );
        assert_eq!(t.audio[0].end, ms(3000.0));
        apply(
            &mut t,
            Edit::ResizeAudio {
                id: outcome.created.unwrap(),
                edge: Edge::End,
                to: ms(9000.0),
            },
        );
        assert_eq!(t.audio[0].end, ms(3000.0));
    }

    #[test]
    fn a_new_audio_item_lands_on_a_new_track() {
        let mut t = timeline();
        apply(
            &mut t,
            Edit::AddAudio {
                item: audio(0.0, 1000.0, 5000.0),
            },
        );
        apply(
            &mut t,
            Edit::AddAudio {
                item: audio(0.0, 1000.0, 5000.0),
            },
        );
        assert_eq!((t.audio[0].track, t.audio[1].track), (0, 1));
    }

    #[test]
    fn audio_stops_at_neighbours_on_its_track() {
        let mut t = timeline();
        let a = apply(
            &mut t,
            Edit::AddAudio {
                item: audio(0.0, 1000.0, 5000.0),
            },
        )
        .created
        .unwrap();
        let b = apply(
            &mut t,
            Edit::AddAudio {
                item: audio(5000.0, 6000.0, 5000.0),
            },
        )
        .created
        .unwrap();
        apply(
            &mut t,
            Edit::MoveAudio {
                id: b.clone(),
                to: ms(5000.0),
                track: Some(0),
            },
        );
        apply(
            &mut t,
            Edit::MoveAudio {
                id: a.clone(),
                to: ms(9000.0),
                track: None,
            },
        );
        let item = t.audio.iter().find(|item| item.id == a).unwrap();
        assert_eq!(item.end, ms(5000.0));
    }

    #[test]
    fn resized_audio_stops_at_neighbours_on_its_track() {
        let mut t = timeline();
        let a = apply(
            &mut t,
            Edit::AddAudio {
                item: audio(0.0, 1000.0, 9000.0),
            },
        )
        .created
        .unwrap();
        let b = apply(
            &mut t,
            Edit::AddAudio {
                item: audio(5000.0, 6000.0, 9000.0),
            },
        )
        .created
        .unwrap();
        apply(
            &mut t,
            Edit::MoveAudio {
                id: b.clone(),
                to: ms(5000.0),
                track: Some(0),
            },
        );
        apply(
            &mut t,
            Edit::ResizeAudio {
                id: a,
                edge: Edge::End,
                to: ms(8000.0),
            },
        );
        assert_eq!(t.audio[0].end, ms(5000.0));
    }

    #[test]
    fn an_overlapping_caption_drop_is_rejected() {
        let mut t = timeline();
        let a = apply(
            &mut t,
            Edit::AddCaption {
                start: ms(0.0),
                end: ms(1000.0),
                text: "a".to_string(),
            },
        )
        .created
        .unwrap();
        let b = apply(
            &mut t,
            Edit::AddCaption {
                start: ms(2000.0),
                end: ms(3000.0),
                text: "b".to_string(),
            },
        )
        .created
        .unwrap();
        assert_eq!(
            t.apply(&Edit::MoveCaption {
                id: a,
                to: ms(2500.0)
            }),
            Err(EditError::WouldOverlap)
        );
        assert_eq!(
            t.apply(&Edit::ResizeCaption {
                id: b,
                edge: Edge::Start,
                to: ms(500.0)
            }),
            Err(EditError::WouldOverlap)
        );
    }

    #[test]
    fn empty_captions_are_dropped() {
        let mut t = timeline();
        let cue = |text: &str| Caption {
            id: "x".into(),
            start: ms(0.0),
            end: ms(1000.0),
            text: text.to_string(),
            words: vec![],
        };
        apply(
            &mut t,
            Edit::SetCaptions {
                captions: vec![cue("   "), cue("hi")],
            },
        );
        assert_eq!(t.captions.len(), 1);
    }

    // ---- Property tests: any sequence of edits leaves a valid timeline. ----

    fn any_edit(ids: Vec<ItemId>) -> impl Strategy<Value = Edit> {
        let id = prop::sample::select(ids);
        let time = (-2_000_i64..60_000).prop_map(|value| TimelineTime::from_ms(value as f64));
        let edge = prop_oneof![Just(Edge::Start), Just(Edge::End)];
        let speed = prop::sample::select(CLIP_SPEEDS.to_vec());
        prop_oneof![
            time.clone().prop_map(|at| Edit::SplitClip { at }),
            (id.clone(), edge.clone(), time.clone()).prop_map(|(id, edge, to)| Edit::TrimClip {
                id,
                edge,
                to
            }),
            (id.clone(), time.clone()).prop_map(|(id, to)| Edit::MoveClip { id, to }),
            (id.clone(), 0_usize..6).prop_map(|(id, to_index)| Edit::ReorderClip { id, to_index }),
            (id.clone(), speed).prop_map(|(id, speed)| Edit::SetClipSpeed { id, speed }),
            id.clone().prop_map(|id| Edit::DeleteClip { id }),
            time.clone().prop_map(|start| Edit::AddZoom {
                start,
                depth: 2,
                focus: Point::CENTER,
                mode: ZoomMode::Auto
            }),
            (id.clone(), time.clone()).prop_map(|(id, to)| Edit::MoveZoom { id, to }),
            (id.clone(), edge.clone(), time.clone()).prop_map(|(id, edge, to)| Edit::ResizeZoom {
                id,
                edge,
                to
            }),
            (id.clone(), edge.clone(), time.clone()).prop_map(|(id, edge, to)| Edit::ResizeAudio {
                id,
                edge,
                to
            }),
            id.clone().prop_map(|id| Edit::DeleteZoom { id }),
            (time.clone(), 100_i64..3000, any::<bool>()).prop_map(|(start, length, _)| {
                Edit::AddCaption {
                    start,
                    end: start + Micros::from_ms(length as f64),
                    text: "x".to_string(),
                }
            }),
            (id.clone(), time.clone()).prop_map(|(id, to)| Edit::MoveCaption { id, to }),
            (id.clone(), edge, time.clone()).prop_map(|(id, edge, to)| Edit::ResizeCaption {
                id,
                edge,
                to
            }),
            (time.clone(), 1000_i64..9000, 100_i64..9000).prop_map(|(start, length, file)| {
                Edit::AddAudio {
                    item: AudioItem {
                        id: "".into(),
                        start,
                        end: start + Micros::from_ms(length as f64),
                        file: crate::project::MediaRef::absolute("/a.wav"),
                        file_duration: Micros::from_ms(file as f64),
                        volume: 1.0,
                        boost: false,
                        track: 0,
                    },
                }
            }),
            (id, time, 0_u32..3).prop_map(|(id, to, track)| Edit::MoveAudio {
                id,
                to,
                track: Some(track)
            }),
        ]
    }

    fn known_ids() -> Vec<ItemId> {
        let mut ids = vec![];
        for prefix in ["clip", "zoom", "caption", "audio"] {
            for n in 1..4 {
                ids.push(ItemId(format!("{prefix}-{n}")));
            }
        }
        ids
    }

    /// A timeline with several of everything, so random edits meet neighbours straight away.
    fn populated() -> Timeline {
        let mut t = Timeline::for_recording(Micros::from_secs(30));
        for at in [10_000.0, 20_000.0] {
            t.apply(&Edit::SplitClip { at: ms(at) }).unwrap();
        }
        for start in [2000.0, 5000.0, 12_000.0] {
            t.apply(&Edit::AddZoom {
                start: ms(start),
                depth: 1,
                focus: Point::CENTER,
                mode: ZoomMode::Auto,
            })
            .unwrap();
        }
        for start in [1000.0, 4000.0, 15_000.0] {
            t.apply(&Edit::AddCaption {
                start: ms(start),
                end: ms(start + 1500.0),
                text: "x".to_string(),
            })
            .unwrap();
        }
        for start in [0.0, 8000.0] {
            t.apply(&Edit::AddAudio {
                item: audio(start, start + 2000.0, 9000.0),
            })
            .unwrap();
        }
        // New audio gets its own track, so share one to make neighbours.
        t.apply(&Edit::MoveAudio {
            id: "audio-2".into(),
            to: ms(8000.0),
            track: Some(0),
        })
        .unwrap();
        t.check().unwrap();
        t
    }

    /// Total footage shown, which edits that don't trim or delete must keep.
    fn footage(timeline: &Timeline) -> Vec<(SourceTime, SourceTime)> {
        let mut ranges: Vec<_> = timeline
            .clips
            .iter()
            .map(|clip| (clip.source_start, clip.source_end()))
            .collect();
        ranges.sort();
        ranges
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn any_sequence_of_edits_leaves_a_valid_timeline(edits in prop::collection::vec(any_edit(known_ids()), 1..40)) {
            let mut timeline = populated();
            for edit in &edits {
                let before = timeline.clone();
                match timeline.apply(edit) {
                    Ok(_) => prop_assert!(timeline.check().is_ok(), "{:?} after {:?}", timeline.check(), edit),
                    Err(_) => prop_assert_eq!(&timeline, &before, "a refused edit changed something: {:?}", edit),
                }
            }
        }

        #[test]
        fn moving_and_reordering_never_change_what_footage_exists(
            splits in prop::collection::vec(1000_i64..29_000, 1..5),
            moves in prop::collection::vec((0_usize..6, 0_usize..6, 0_i64..60_000, any::<bool>()), 1..20),
        ) {
            let mut timeline = Timeline::for_recording(Micros::from_secs(30));
            for split in splits {
                let _ = timeline.apply(&Edit::SplitClip { at: TimelineTime::from_ms(split as f64) });
            }
            let expected = footage(&timeline);
            for (clip, index, to, reorder) in moves {
                let id = timeline.clips[clip % timeline.clips.len()].id.clone();
                let edit = if reorder {
                    Edit::ReorderClip { id, to_index: index }
                } else {
                    Edit::MoveClip { id, to: TimelineTime::from_ms(to as f64) }
                };
                let _ = timeline.apply(&edit);
                prop_assert!(timeline.check().is_ok());
                prop_assert_eq!(&footage(&timeline), &expected);
            }
        }

        #[test]
        fn splitting_never_changes_what_shows_at_any_time(
            at in 200_i64..29_800,
            speed in prop::sample::select(CLIP_SPEEDS.to_vec()),
            probes in prop::collection::vec(0.0_f64..1.0, 20),
        ) {
            let mut timeline = Timeline::for_recording(Micros::from_secs(30));
            timeline.apply(&Edit::SetClipSpeed { id: "clip-1".into(), speed }).unwrap();
            let before = timeline.clone();
            let duration = before.clips[0].duration;
            let at = TimelineTime(duration.0 * at / 30_000);
            if timeline.apply(&Edit::SplitClip { at }).is_ok() {
                for probe in probes {
                    let time = TimelineTime((duration.0 as f64 * probe) as i64);
                    if let (Some(a), Some(b)) = (before.to_source(time), timeline.to_source(time)) {
                        prop_assert!(((a.0 - b.0).abs() as f64) <= speed.max(1.0) + 1.0);
                    }
                }
                prop_assert_eq!(timeline.clips[0].end(), timeline.clips[1].start);
            }
        }
    }
}
