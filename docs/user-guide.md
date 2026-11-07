# Using Reel

## Recording

Reel opens as a small bar at the bottom of your screen.

1. Click **Screen** to choose a display or a window. The first time, macOS asks for Screen Recording permission. After you allow it in System Settings, quit Reel and open it again, which macOS requires.
2. The microphone button turns system audio on and off and chooses a mic. The camera button chooses a webcam. They're asked for only when you pick them.
3. The timer button sets a countdown of 0, 3, 5 or 10 seconds.
4. Click **Record**, or press **⌥⌘R** from any app.

While recording, you can pause, stop, or hide the bar. The × discards the recording, and asks first. Press ⌥⌘R again to stop. Paused time is left out of the video, the sound and the cursor data.

When you stop, the recording opens in the editor. Closing the editor brings the bar back.

If Reel or your Mac crashes during a recording, what was recorded is saved. It's offered the next time you open Reel, with at most about a second lost.

## Editing

The editor has a preview, a timeline below it, and settings on the left. Selecting something on the timeline shows that item's settings.

**The timeline.** Click the ruler to move the playhead. Drag an item to move it and its edges to resize it. ⌘-scroll zooms the timeline, and shift-scroll pans it.

| Key | Does |
|---|---|
| Space | Play or pause |
| C | Split the clip at the playhead |
| Z | Add a zoom |
| A | Add a text annotation |
| F | Add a keyframe marker, which the playhead snaps to and the skip buttons stop at |
| ⌘D or Delete | Delete what's selected |
| ⌘A | Select all zooms |
| Tab, ⇧Tab | Move between the annotations under the playhead |
| ⌘Z, ⇧⌘Z | Undo and redo |
| ⌘S, ⇧⌘S | Save and Save As |
| ⌘O | Your projects |
| Esc | Clear the selection or close a panel |

You can change the first six in **Settings → Keyboard shortcuts**.

**Clips.** Split a clip to cut something out, then delete the part you don't want. Playback and export skip the gap. A clip's panel sets its speed, and **Move earlier** and **Move later** reorder clips, with their footage.

**Zooms.** Add a zoom at the playhead with Z, or hover over the zoom row and click. By default it follows the cursor. Choose **Fixed spot** to aim it yourself by clicking or dragging on the preview. The wand adds zooms around clusters of clicks.

**Annotations.** **Add** has text, arrows, blur and images. Drag one on the preview to move it, and by a corner to resize it. A blur can have a solid fill, for hiding something completely.

**Scene.** Background, frame shadow and corners, padding and crop. **Crop** above the preview shows the whole video with edges to drag.

**Cursor.** The cursor is drawn from recorded data, so its style and size, its click effects and how it moves can all be changed afterwards.

**Webcam.** Position, size, shape and mirroring. While this panel is open, you can drag the bubble on the preview. If the webcam is out of step with the recording, **Sync offset** corrects it.

**Audio.** **Add → Audio** adds a sound on its own row. System audio and your mic are separate tracks with separate volumes, in a clip's panel.

**Presets.** Saves the look of a project, without its timeline, to use on others.

Projects save themselves about a second after each change.

## Captions

In the **Captions** panel, click **Download model** the first time (466 MB), then **Generate**. It runs on your Mac, and nothing is uploaded. It uses your mic track when there is one, and **Sound to use** changes that.

Select a caption on the timeline to change its text and times, split it at the playhead, or merge it with the next. Hover over the caption row and click to add one by hand.

## Exporting

**Export** at the top right has MP4 and GIF. When it's done, choose where to save it. Cancelling that dialog keeps the render, and you can save as many copies as you like until you close the panel. An MP4 can also save `.srt` and `.vtt` caption files beside it.

## Where things are

Everything is in `~/Library/Application Support/Reel/`: `recordings/`, `settings.json`, `models/` and `logs/`. Logs stay on your Mac, and are deleted after two weeks. Reel only uses the internet to download a speech model when you ask it to.

Old recordings are never deleted unless you turn on **Keep old recordings for 14 days only** in Settings.
