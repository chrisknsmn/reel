# Vendored code

## Rust/UI

| | |
|---|---|
| **Source** | https://github.com/rust-ui/ui |
| **Commit** | `4f5a516e76d75a15a587ab2169352cf34fd318d0` (30 Oct 2026) |
| **Licence** | MIT, reproduced below |
| **Copied on** | 6 Nov 2026 |

### Copied unchanged

| In Reel | From upstream |
|---|---|
| `crates/reel-app/src/ui/*.rs`, except `mod.rs` | `app_crates/registry/src/ui/` |
| `crates/reel-app/src/hooks/*.rs`, except `mod.rs` | `app_crates/registry/src/hooks/` |
| `crates/reel-app/assets/rust-ui/toast.js`, `toast.css` | `public/app_components/` |
| `crates/reel-app/style/slider.css` | `style/slider.css` |

Keep these files byte-for-byte unless a milestone calls for a change, and record any change here. `main.rs` exempts them from `rustfmt` and Clippy so upstream diffs stay readable.

**Components:** Accordion, Alert Dialog, Button, Card, Context Menu, Dialog, Dropdown Menu, Input, Kbd, Label, Popover, Progress, Scroll Area, Select, Separator, Skeleton, Slider, Spinner, Switch, Tabs, Toast, Toggle Group, Tooltip.

**Hooks:** `use_random` and `use_can_scroll_vertical`, the only two these components import.

### Written for Reel

- `ui/mod.rs` and `hooks/mod.rs` list only the files above.
- `crates/reel-app/tailwind.css` is adapted from upstream's `tailwind.css`:
  - The colour and radius tokens and the `dark` variant are unchanged, minus the chart, sidenav and surface tokens.
  - The utilities for bottom navigation, chat and iOS safe areas, the `tw-animate-css` import and the other website styles are left out, because no vendored component uses them. Add `tw-animate-css` back if a later component needs it.
  - The font is the system font rather than Inter, which isn't bundled.
- Dependencies are narrower than upstream's: no `fullstack` feature on `dioxus`, no `dioxus_animated` on `icons`, no `debug` on `tw_merge`.

### Updating

1. Diff the files above against a newer upstream commit, then copy across what you want.
2. Update the commit here and re-apply any changes recorded in this file.
3. Run `dx build --platform desktop` to regenerate `assets/tailwind.css`, and commit the result.

Dioxus is pinned to `=0.7.9`, the same version Rust/UI's manifest pins at this commit. Upstream's comment says that's because `dioxus-desktop` 0.7.10 is unavailable, but it was published on 30 Jul 2026, and upstream's own lockfile mixes 0.7.9 and 0.7.10 crates. Reel's lockfile holds every Dioxus crate at 0.7.9, and `deny.toml` enforces it.

### Licence

```text
MIT License

Copyright (c) 2026 Max Wells

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
