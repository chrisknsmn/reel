//! A scripted run through the editor: real clicks, keys and drags on the real page, then checking what
//! they did to the project, what's on screen, and what's on disk.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dioxus::desktop::tao::window::Window;
use dioxus::prelude::*;
use objc2_app_kit::{NSEventModifierFlags, NSEventType};
use reel_core::format;
use reel_core::history::Selection;
use reel_core::time::TimelineTime;
use serde_json::{Value, json};

use crate::editor::Ed;
use crate::native::ns_window;
use crate::self_test::{capture, send_key_through_app, send_mouse_kind};

async fn sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

async fn eval(script: &str) -> Value {
    document::eval(script).await.unwrap_or(Value::Null)
}

async fn center(selector: &str) -> Option<(f64, f64)> {
    let value = eval(&format!(
        "const e = document.querySelector({selector:?}); if (!e) return null; const r = e.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];"
    ))
    .await;
    Some((value.get(0)?.as_f64()?, value.get(1)?.as_f64()?))
}

async fn rect(selector: &str) -> Option<(f64, f64, f64, f64)> {
    let value = eval(&format!(
        "const e = document.querySelector({selector:?}); if (!e) return null; const r = e.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];"
    ))
    .await;
    Some((
        value.get(0)?.as_f64()?,
        value.get(1)?.as_f64()?,
        value.get(2)?.as_f64()?,
        value.get(3)?.as_f64()?,
    ))
}

async fn exists(selector: &str) -> bool {
    center(selector).await.is_some()
}

fn from_bottom(window: &Window, y: f64) -> f64 {
    ns_window(window)
        .contentView()
        .map_or(0.0, |view| view.frame().size.height)
        - y
}

async fn click_at(window: &Window, x: f64, y: f64) {
    let up_from_bottom = from_bottom(window, y);
    send_mouse_kind(window, NSEventType::LeftMouseDown, x, up_from_bottom);
    sleep(30).await;
    send_mouse_kind(window, NSEventType::LeftMouseUp, x, up_from_bottom);
    sleep(150).await;
}

async fn click(window: &Window, selector: &str) -> bool {
    let Some((x, y)) = center(selector).await else {
        return false;
    };
    click_at(window, x, y).await;
    true
}

/// A real drag: press, move a few steps, let go.
async fn drag(window: &Window, from: (f64, f64), to: (f64, f64)) {
    send_mouse_kind(
        window,
        NSEventType::LeftMouseDown,
        from.0,
        from_bottom(window, from.1),
    );
    sleep(60).await;
    for step in 1..=6 {
        let t = f64::from(step) / 6.0;
        let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        send_mouse_kind(
            window,
            NSEventType::LeftMouseDragged,
            x,
            from_bottom(window, y),
        );
        sleep(30).await;
    }
    send_mouse_kind(
        window,
        NSEventType::LeftMouseUp,
        to.0,
        from_bottom(window, to.1),
    );
    sleep(200).await;
}

const COMMAND: NSEventModifierFlags = NSEventModifierFlags::Command;

async fn press(window: &Window, text: &str, code: u16) {
    send_key_through_app(window, text, code, NSEventModifierFlags::empty());
    sleep(150).await;
}

async fn press_command(window: &Window, text: &str, code: u16, shift: bool) {
    let flags = if shift {
        COMMAND | NSEventModifierFlags::Shift
    } else {
        COMMAND
    };
    // A real shifted key reports its capital.
    let typed = if shift {
        text.to_uppercase()
    } else {
        text.to_string()
    };
    send_key_through_app(window, &typed, code, flags);
    sleep(150).await;
}

async fn focus_editor() {
    eval("document.getElementById('editor-root').focus(); return true;").await;
    sleep(30).await;
}

/// Sets a slider the way dragging it does.
async fn set_slider(selector: &str, value: f64) {
    eval(&format!(
        "const i = document.querySelector({selector:?}); i.value = {value}; i.dispatchEvent(new Event('input', {{ bubbles: true }})); i.dispatchEvent(new MouseEvent('mouseup', {{ bubbles: true }})); return true;"
    ))
    .await;
    sleep(150).await;
}

/// How different a part of two captures is.
fn difference(
    a: &crate::self_test::Capture,
    b: &crate::self_test::Capture,
    area: (usize, usize, usize, usize),
) -> u64 {
    let mut total = 0;
    for y in (area.1..area.1 + area.3).step_by(7) {
        for x in (area.0..area.0 + area.2).step_by(7) {
            let (p, q) = (a.pixel(x, y), b.pixel(x, y));
            total += (0..3).map(|c| u64::from(p[c].abs_diff(q[c]))).sum::<u64>();
        }
    }
    total
}

pub async fn run(ed: Ed) {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(150));
        eprintln!("self-test timed out");
        std::process::exit(3);
    });
    let out = PathBuf::from(std::env::var("REEL_OUT").unwrap_or_else(|_| ".".into()));
    std::fs::create_dir_all(&out).expect("couldn't create the output folder");
    let context = dioxus::desktop::window();
    let window = context.window.clone();
    window.set_always_on_top(true);
    crate::native::bring_forward(&window);
    let mut checks: Vec<Value> = vec![];
    let mut check = |name: &str, pass: bool, detail: String| {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        checks.push(json!({ "name": name, "pass": pass, "detail": detail }));
    };
    let shot = |name: &str| {
        if let Some(image) = capture(&window) {
            image.save(&out.join(format!("{name}.png")));
        }
    };
    let project = || ed.project().expect("a project should be open");
    sleep(1500).await;

    check(
        "the editor is open on a project",
        ed.project().is_some(),
        String::new(),
    );
    check(
        "the layout has every part",
        exists("#side-panel").await
            && exists("#timeline").await
            && exists("#preview-hole").await
            && exists(".ed-rail").await,
        String::new(),
    );
    shot("editor-1-opened");

    // Clicking the ruler moves the playhead.
    let (rx, ry, rw, rh) = rect("#timeline-ruler").await.expect("a ruler");
    let total = project().timeline.duration();
    let at_quarter = (rx + rw * 0.25, ry + rh / 2.0);
    click_at(&window, at_quarter.0, at_quarter.1).await;
    sleep(200).await;
    let moved = (ed.now().secs() - total.secs() * 0.25).abs();
    check(
        "clicking the ruler moves the playhead",
        moved < 0.3,
        format!("{:.2} s of {:.2} s", ed.now().secs(), total.secs()),
    );

    // Split, from the keyboard, then undo and redo.
    focus_editor().await;
    let before = project().timeline.clips.len();
    press(&window, "c", 8).await;
    check(
        "C splits the clip at the playhead",
        project().timeline.clips.len() == before + 1,
        format!("{} clips", project().timeline.clips.len()),
    );
    press_command(&window, "z", 6, false).await;
    check(
        "Cmd+Z undoes it",
        project().timeline.clips.len() == before,
        format!("{} clips", project().timeline.clips.len()),
    );
    press_command(&window, "z", 6, true).await;
    check(
        "Cmd+Shift+Z redoes it",
        project().timeline.clips.len() == before + 1,
        format!("{} clips", project().timeline.clips.len()),
    );

    // Trim by dragging a clip's edge.
    let second = project().timeline.clips[1].clone();
    let selector = format!("#item-{} .tl-cap-start", second.id.0);
    if let Some((x, y)) = center(&selector).await {
        drag(&window, (x, y), (x + rw * 0.1, y)).await;
    }
    let trimmed = project().timeline.clips[1].clone();
    check(
        "dragging an edge trims the clip",
        trimmed.start > second.start,
        format!(
            "{:.2} s, was {:.2} s",
            trimmed.start.secs(),
            second.start.secs()
        ),
    );
    check(
        "and it's selected",
        ed.selection() == Selection::Clip(second.id.clone()),
        format!("{:?}", ed.selection()),
    );
    check(
        "selecting a clip shows its panel",
        exists("#delete-selected").await,
        String::new(),
    );
    shot("editor-2-clip-selected");
    // One whole drag is one undo step, however many moves it took.
    press_command(&window, "z", 6, false).await;
    check(
        "one drag is one undo step",
        project().timeline.clips[1].start == second.start,
        format!("{:.2} s", project().timeline.clips[1].start.secs()),
    );
    press_command(&window, "z", 6, true).await;

    // Move a clip by dragging its body.
    let moving = project().timeline.clips[1].clone();
    if let Some((x, y)) = center(&format!("#item-{}", moving.id.0)).await {
        drag(&window, (x, y), (x + rw * 0.05, y)).await;
    }
    check(
        "dragging a clip moves it",
        project().timeline.clips[1].start > moving.start,
        format!(
            "{:.2} s, was {:.2} s",
            project().timeline.clips[1].start.secs(),
            moving.start.secs()
        ),
    );

    // Speed, from the clip's panel.
    if click(&window, ".row-speeds button:nth-child(6)").await {
        check(
            "the speed buttons work",
            (project().timeline.clips[1].speed - 1.5).abs() < 0.01,
            format!("{}×", project().timeline.clips[1].speed),
        );
    }
    press_command(&window, "z", 6, false).await;

    // Real reordering: the footage goes with the clip, which Recordly can't do (§15).
    ed.select(Selection::Clip(project().timeline.clips[0].id.clone()));
    sleep(150).await;
    let first_footage = project().timeline.clips[0].source_start;
    let second_footage = project().timeline.clips[1].source_start;
    click(&window, "#clip-later").await;
    let reordered = project();
    check(
        "a clip can be moved later, with its footage",
        reordered.timeline.clips[0].source_start == second_footage
            && reordered.timeline.clips[1].source_start == first_footage,
        format!(
            "{:?}",
            reordered
                .timeline
                .clips
                .iter()
                .map(|clip| clip.source_start.secs())
                .collect::<Vec<_>>()
        ),
    );
    press_command(&window, "z", 6, false).await;
    check(
        "and undo puts it back",
        project().timeline.clips[0].source_start == first_footage,
        String::new(),
    );
    ed.select(Selection::Clip(project().timeline.clips[1].id.clone()));
    sleep(150).await;

    // Delete from the keyboard, which Recordly can't do for clips (§15, §58).
    focus_editor().await;
    let count = project().timeline.clips.len();
    press(&window, "", 51).await;
    send_key_through_app(&window, "\u{7f}", 51, NSEventModifierFlags::empty());
    sleep(150).await;
    check(
        "Delete removes the selected clip",
        project().timeline.clips.len() == count - 1,
        format!("{} clips", project().timeline.clips.len()),
    );
    press_command(&window, "z", 6, false).await;
    check(
        "and undo brings it back",
        project().timeline.clips.len() == count,
        format!("{} clips", project().timeline.clips.len()),
    );
    sleep(100).await;
    let whole_length = project().timeline.duration();
    check(
        "the view follows the timeline's new length",
        (ed.view)().length == whole_length.max(crate::editor::SMALLEST_VIEW),
        format!(
            "{:.1} s of {:.1} s",
            (ed.view)().length.secs(),
            whole_length.secs()
        ),
    );

    // A zoom, from the keyboard, then its panel.
    click_at(&window, rx + rw * 0.05, ry + rh / 2.0).await;
    focus_editor().await;
    let zooms = project().timeline.zooms.len();
    press(&window, "z", 6).await;
    check(
        "Z adds a zoom",
        project().timeline.zooms.len() == zooms + 1,
        format!("{} zooms", project().timeline.zooms.len()),
    );
    check(
        "and shows its panel",
        exists("#zoom-depth-3").await,
        String::new(),
    );
    click(&window, "#zoom-depth-3").await;
    let newest = project()
        .timeline
        .zooms
        .iter()
        .find(|zoom| zoom.depth == 3)
        .cloned();
    check("the depth buttons work", newest.is_some(), String::new());
    shot("editor-3-zoom");

    // An annotation, from the keyboard, then typing into it and dragging it on the preview.
    let annotations = project().timeline.annotations.len();
    focus_editor().await;
    press(&window, "a", 0).await;
    check(
        "A adds an annotation",
        project().timeline.annotations.len() == annotations + 1,
        format!("{}", project().timeline.annotations.len()),
    );
    eval("const t = document.getElementById('annotation-text'); t.value = 'Hello from the test'; t.dispatchEvent(new Event('input', { bubbles: true })); return true;").await;
    sleep(200).await;
    let added = project()
        .timeline
        .annotations
        .last()
        .cloned()
        .expect("an annotation");
    let says_it = matches!(&added.content, reel_core::timeline::AnnotationContent::Text { text, .. } if text == "Hello from the test");
    check("typing changes its text", says_it, String::new());
    sleep(300).await;
    shot("editor-4-annotation");
    if let Some((x, y)) = center(".ph-annotation[data-selected='true']").await {
        drag(&window, (x, y), (x + 60.0, y + 30.0)).await;
    }
    let placed = project()
        .timeline
        .annotations
        .last()
        .cloned()
        .expect("an annotation");
    check(
        "dragging it on the preview moves it",
        placed.position.x > added.position.x && placed.position.y > added.position.y,
        format!("{:?}", placed.position),
    );
    if let Some((x, y)) = center(".ph-grip-br").await {
        drag(&window, (x, y), (x + 40.0, y + 20.0)).await;
    }
    let sized = project()
        .timeline
        .annotations
        .last()
        .cloned()
        .expect("an annotation");
    check(
        "dragging a corner resizes it",
        sized.width > placed.width && sized.height > placed.height,
        format!("{}×{}", sized.width, sized.height),
    );

    // Every rail panel opens.
    press(&window, "\u{1b}", 53).await;
    ed.select(Selection::None);
    sleep(100).await;
    for name in ["Cursor", "Webcam", "Captions", "Settings", "Scene"] {
        let opened = click(&window, &format!("#rail-{name}")).await;
        sleep(150).await;
        let shown = eval("return document.querySelector('#side-panel h2').textContent;").await;
        check(
            &format!("the {name} panel opens"),
            opened && shown.as_str() == Some(name),
            format!("{shown}"),
        );
        shot(&format!("editor-5-panel-{name}"));
    }

    // A look change reaches the preview, and a whole drag of a slider is one undo step.
    let original_shadow = project().look.shadow;
    let before_image = capture(&window);
    for value in [0.1, 0.3, 0.5, 0.15] {
        eval(&format!(
            "const i = document.querySelector('#slider-shadow input'); i.value = {value}; i.dispatchEvent(new Event('input', {{ bubbles: true }})); return true;"
        ))
        .await;
        sleep(40).await;
    }
    set_slider("#slider-shadow input", 0.15).await;
    check(
        "a slider changes the look",
        (project().look.shadow - 0.15).abs() < 0.001,
        format!("{}", project().look.shadow),
    );
    focus_editor().await;
    press_command(&window, "z", 6, false).await;
    check(
        "and one undo puts it all back",
        (project().look.shadow - original_shadow).abs() < 0.001,
        format!("{}", project().look.shadow),
    );

    // A background colour reaches the actual pixels.
    click(&window, ".row-swatch[aria-label='#FF0000']").await;
    sleep(700).await;
    let after_image = capture(&window);
    if let (Some(a), Some(b), Some((hx, hy, hw, hh))) =
        (&before_image, &after_image, rect("#preview-hole").await)
    {
        let scale = a.width as f64
            / ns_window(&window)
                .contentView()
                .map_or(1.0, |view| view.frame().size.width);
        let area = (
            (hx * scale) as usize,
            ((hy + (a.height as f64 / scale
                - ns_window(&window)
                    .contentView()
                    .map_or(0.0, |view| view.frame().size.height)))
                * scale) as usize,
            (hw * scale) as usize,
            (hh * scale) as usize,
        );
        let changed = difference(a, b, area);
        check(
            "changing the background changes the picture",
            changed > 10_000,
            format!("{changed}"),
        );
    }
    shot("editor-6-red-background");
    check(
        "and it's in the project",
        matches!(&project().look.background, reel_core::project::Background::Color(color) if color == "#FF0000"),
        String::new(),
    );
    press_command(&window, "z", 6, false).await;

    // The crop editor keeps its result as one undo step, and cancel changes nothing.
    let before_crop = project().look.crop;
    click(&window, "#open-crop").await;
    sleep(400).await;
    check(
        "the crop editor opens",
        exists("#crop-box").await,
        String::new(),
    );
    shot("editor-7-crop");
    click(&window, "#crop-cancel").await;
    check(
        "cancelling leaves the crop alone",
        project().look.crop == before_crop,
        String::new(),
    );
    click(&window, "#open-crop").await;
    sleep(400).await;
    if let Some((x, y)) = center("#crop-left").await {
        drag(&window, (x, y), (x + 120.0, y)).await;
    }
    click(&window, "#crop-done").await;
    sleep(400).await;
    let cropped = project().look.crop;
    check(
        "dragging an edge and pressing Done crops",
        cropped.x > 0.01,
        format!("{cropped:?}"),
    );
    focus_editor().await;
    press_command(&window, "z", 6, false).await;
    check(
        "and the crop can be undone, which Recordly's can't",
        project().look.crop == before_crop,
        String::new(),
    );

    // Keyboard bits.
    focus_editor().await;
    press(&window, "f", 3).await;
    check(
        "F adds a keyframe",
        (ed.keyframes)().len() == 1,
        format!("{}", (ed.keyframes)().len()),
    );
    let playing_before = crate::preview::SHARED.is_playing();
    press(&window, " ", 49).await;
    check(
        "Space plays",
        crate::preview::SHARED.is_playing() != playing_before,
        String::new(),
    );
    press(&window, " ", 49).await;
    press_command(&window, "a", 0, false).await;
    check(
        "Cmd+A selects all zooms",
        ed.selection() == Selection::AllZooms,
        format!("{:?}", ed.selection()),
    );
    press(&window, "\u{1b}", 53).await;
    check(
        "Esc clears the selection",
        ed.selection() == Selection::None,
        format!("{:?}", ed.selection()),
    );

    // The shortcut editor.
    click(&window, "#rail-Settings").await;
    click(&window, "#open-shortcuts").await;
    check(
        "the shortcut editor opens",
        exists("#shortcut-add_zoom").await,
        String::new(),
    );
    shot("editor-8-shortcuts");
    click(&window, "#shortcuts-reset").await;
    click(&window, "#shortcuts-save").await;

    // Presets and the project list open, and an export really runs.
    click(&window, "#open-presets").await;
    check("presets open", exists("#preset-name").await, String::new());
    shot("editor-9-presets");
    click(&window, "#open-projects").await;
    check(
        "the project list opens",
        exists("#project-browser").await,
        String::new(),
    );
    click(&window, "#project-browser button").await;
    press(&window, "\u{1b}", 53).await;
    let mut overlay = ed.overlay;
    overlay.set(crate::editor::CLOSED);
    sleep(100).await;

    // A file that can't be opened says so here, and leaves what's open alone.
    crate::links::tell_editor(crate::links::ToEditor::Open(
        "/no/such/folder/video.mp4".into(),
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut said = (ed.notice)().unwrap_or_default();
    while !said.contains("couldn't be opened") && Instant::now() < deadline {
        sleep(50).await;
        said = (ed.notice)().unwrap_or_default();
    }
    check(
        "a file that can't be opened says so, with the project still open",
        said.contains("couldn't be opened") && ed.project().is_some() && window.is_visible(),
        said,
    );

    click(&window, "#open-export").await;
    check(
        "the export panel opens",
        exists("#export-start").await,
        String::new(),
    );
    shot("editor-10-export");
    click(&window, "#export-start").await;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut saw_progress = false;
    while Instant::now() < deadline {
        saw_progress |= exists("#export-progress").await;
        if exists("#export-done").await || exists("#export-error").await {
            break;
        }
        sleep(50).await;
    }
    check("an export shows its progress", saw_progress, String::new());
    let error =
        eval("const e = document.getElementById('export-error'); return e ? e.textContent : '';")
            .await;
    check(
        "and finishes",
        exists("#export-done").await,
        format!("{error}"),
    );
    shot("editor-11-exported");
    click(&window, "#export-close").await;

    // The webcam can be dragged on the preview, from its panel.
    click(&window, "#rail-Webcam").await;
    sleep(200).await;
    if let Some((x, y)) = center("#webcam-handle").await {
        drag(&window, (x, y), (x - 300.0, y - 120.0)).await;
    }
    let webcam = project().webcam;
    check(
        "dragging the webcam moves it",
        webcam.position == reel_core::project::WebcamPosition::Custom && webcam.custom_x < 0.95,
        format!(
            "{:?} at {:.2},{:.2}",
            webcam.position, webcam.custom_x, webcam.custom_y
        ),
    );
    focus_editor().await;
    press_command(&window, "z", 6, false).await;

    // Captions, through the panel, from real speech on the mic track.
    if std::env::var_os("REEL_TEST_SPEECH").is_some() {
        click(&window, "#rail-Captions").await;
        sleep(200).await;
        let before_captions = capture(&window);
        check(
            "the model is found",
            exists("#generate-captions").await && !exists("#download-model").await,
            String::new(),
        );
        click(&window, "#generate-captions").await;
        let deadline = Instant::now() + Duration::from_secs(60);
        while project().timeline.captions.is_empty() && Instant::now() < deadline {
            sleep(100).await;
        }
        let captions = project().timeline.captions;
        let text = captions
            .iter()
            .map(|caption| caption.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        check(
            "generating makes captions from the mic",
            text.contains("hello") && text.contains("test"),
            text.clone(),
        );
        check(
            "and turns them on",
            project().caption_style.show,
            String::new(),
        );
        check(
            "they show on the timeline",
            exists("#row-captions .tl-caption").await,
            String::new(),
        );
        // Go to the first one, where it should be drawn into the picture.
        if let Some(first) = captions.first() {
            ed.seek(TimelineTime((first.start.0 + first.end.0) / 2));
            sleep(600).await;
            if let (Some(a), Some(b), Some((hx, hy, hw, hh))) = (
                &before_captions,
                &capture(&window),
                rect("#preview-hole").await,
            ) {
                let scale = a.width as f64
                    / ns_window(&window)
                        .contentView()
                        .map_or(1.0, |view| view.frame().size.width);
                let title = a.height as f64 / scale
                    - ns_window(&window)
                        .contentView()
                        .map_or(0.0, |view| view.frame().size.height);
                // The bottom of the preview, where a caption goes.
                let area = (
                    (hx * scale) as usize,
                    ((hy + title + hh * 0.8) * scale) as usize,
                    (hw * scale) as usize,
                    (hh * 0.2 * scale) as usize,
                );
                check(
                    "and in the picture",
                    difference(a, b, area) > 3000,
                    format!("{}", difference(a, b, area)),
                );
            }
            shot("editor-12-captions");
        }
    }

    // Saving, then reading it back from disk.
    focus_editor().await;
    press_command(&window, "s", 1, false).await;
    sleep(300).await;
    let saved_path = ed
        .session
        .read()
        .as_ref()
        .and_then(|session| session.path().map(std::path::Path::to_path_buf));
    let current = project();
    let from_disk = saved_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| format::from_str(&text).ok());
    check(
        "what's saved is what's on screen",
        from_disk
            .as_ref()
            .is_some_and(|saved| saved.timeline == current.timeline && saved.look == current.look),
        format!("{saved_path:?}"),
    );
    check(
        "there's nothing unsaved",
        ed.session
            .read()
            .as_ref()
            .is_some_and(|session| !session.is_dirty()),
        String::new(),
    );
    // Autosave catches a change without being asked.
    ed.change("test-autosave", |p| p.look.corner_radius = 33.0);
    ed.end_gesture();
    sleep(1600).await;
    let autosaved = saved_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| format::from_str(&text).ok());
    check(
        "autosave catches changes a moment later",
        autosaved.is_some_and(|saved: reel_core::project::Project| {
            (saved.look.corner_radius - 33.0).abs() < 0.001
        }),
        String::new(),
    );

    let report =
        json!({ "checks": checks, "passed": checks.iter().all(|check| check["pass"] == true) });
    let passed = report["passed"] == true;
    let _ = std::fs::write(out.join("report.json"), report.to_string());
    println!(
        "{}",
        if passed {
            "self-test passed"
        } else {
            "self-test failed"
        }
    );
    sleep(200).await;
    std::process::exit(i32::from(!passed));
}
