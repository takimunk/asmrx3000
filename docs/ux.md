# ASMR X3000 — UX notes

## 1. Audit of the popover before the overhaul

Snapshots: `docs/ux/before/` (static, selected bird, expanded water branch, flight with gaze,
prompt reply, menu bar frames).

**Hierarchy**
- Everything has the same weight. Scene, prompt, spectrogram and ten waveform rows all shout
  in the same white-on-black stripes; nothing says "start here".
- The scene is ~45 % of the height; the spectrogram plus the track list take the other half
  and are the busiest part of the screen.
- The prompt — the main way to steer the scene — is a grey placeholder on a hairline,
  sandwiched between the scene's caption and an ornament. It reads as a footnote.

**Discoverability of buttons**
- The transport (pause, fly, recenter, head tracking, quit) is five 17 pt grey line icons in
  the top right corner, on top of a busy engraving, behind a radial black smudge. At 55 %
  grey they disappear into the trees. No labels, no tooltips, no visible hit area.
- Quit sits next to recenter with the same weight: a mis-click closes the app.
- The open/close mark of a track's branch is a 3.5 pt lozenge under the pictogram. Nobody
  will find it.

**Sliders (the "mush")**
- Ten macro rows, 23 pt each, all drawn as full-width waveform barcodes. The knob is a 5 pt
  hollow diamond on a 1 pt stem; the value is only legible as "where the bright part stops".
- Rows without sound (rain, environment, insects, fire, flight) are dotted lines — they look
  disabled even though they are not.
- Expanding a group inserts 9 child sliders inline, pushes every other row down and scrolls
  the list: the tangle in `expanded.png`. Children have no names until hovered; nine
  identical hairline scales with ticks are indistinguishable.
- Pictograms are 18 pt engraved line art at 55 % grey; several are illegible at that size
  (foliage, environment, flight, master).

**Legibility**
- Body text is 11–11.5 pt grey on a striped engraving (`Click a tree…`, the selection help,
  the orientation labels). Contrast is well under 4.5:1 against the stripes.
- Spectrogram labels and the hover readout (`1.64 kHz, −62 dB, 2.8 s ago`) are the brightest
  text in the popover although they matter least.

**Scene**
- The engraving shader cuts every surface into 1–2 px lines, adds cross-hatching, flicked
  foliage, dot-and-lick ground, wandering air strokes, watercolour washes off register and
  grain. At popover size it turns into moiré and shimmering noise; the ground is a field of
  horizontal stripes that fight the text laid over it.
- Fire and washes add warm colour, so the violet listener is not the only vivid colour.

**States**
- Prompt: no visible send affordance, no loading state beyond "слушаю…", errors are the same
  grey line as replies. Empty state is just the placeholder.
- No hover / pressed / focus states anywhere except a colour change of the icon.
- Toggles (head tracking, flight) show state only by a strike-through or a brighter line.

**Keyboard and focus**
- All buttons are `.focusable(false)`; the only keyboard path is the scene (arrows, tab,
  esc, space). There is no way to reach the prompt from the keyboard except by clicking.
- No focus ring anywhere.

**Motion**
- Every pictogram animates whenever its sound plays; ten icons wiggle at once. Reduced
  motion is not respected (icons, menu bar wave).

**Menu bar**
- The wordmark is 66 × 22 pt: three to four normal menu bar icons wide.

## 2. Direction

Primary task: **listen, and steer the forest** — by a sentence in the prompt, by a few big
macro levers, by moving things in the scene. Secondary: fine physical parameters and
analysis (spectrogram).

```
┌─────────────────────────────────────────────┐
│                                             │
│                                             │
│                SCENE (dominant)             │
│                                             │
│ ◦ selection chip / hint                     │
├─────────────────────────────────────────────┤
│ ▶  ─────○ vol     ✈  ◎  ◖◗        09:00   ⏻ │  transport: one row, labels on hover
├─────────────────────────────────────────────┤
│ X_ Скажите, каким быть лесу…            ⏎   │  prompt: framed, the second loudest thing
│    reply / error / thinking line            │
├─────────────────────────────────────────────┤
│ ≋  water   ━━━━━━━━━━━━━━━━━○              › │
│ ⌇  wind    ━━━━━○                          › │  4–6 macro levers, pictogram + slider
│ ♪  birds   ━━━━━━━━━━━━○                   › │  the waveform is the slider's fill
│ ⁄⁄ rain    ○                               › │
│ ☼  time    ━━━━━━━━━○                      › │
│ ··· more (foliage, insects, fire, flight)    │
├─────────────────────────────────────────────┤
│ ▁▂▃▂▁▁▂▅▃▂ spectrogram strip (click to open) │
└─────────────────────────────────────────────┘
```

Details of a group open in a **drawer** that replaces the lever list (one group at a time,
`‹` or esc to go back), with each parameter named and its value shown.

See section 3 for what was built.

## 3. What was built

Snapshots: `docs/ux/after/` (static, selected tree, water drawer with the spectrogram open,
flight with gaze, prompt reply at night, all levers, night). Menu bar: `docs/ux/icon/`.

**Tokens** (`Panel/Icons.swift`)
- Colour: `Ink.panel` 0.078 grey (the strips' shaders use the same value), `raised` 0.13
  (fields, hover), `pressed` 0.19, `hairline` 0.21, `paper` 0.93 (primary), `graphite` 0.62
  (secondary), `ash` 0.42 (hints, disabled), `violet` (#8754FF) for the listener only.
- Type: Coppice Mono only, four sizes: caption 11, body 12, title 13, prompt 14.
- Spacing: `Grid` on a 4 pt base; 16 pt gutters, 28 pt rows, 32 pt buttons, 8 pt radii on
  controls, 6 pt on chips.
- Motion: `Motion.reduced` (system "Reduce motion") stops icon animation, the menu bar wave
  (letters are simply blue), the prompt cursor blink and the thinking dots. Icons animate only
  while hovered, never ten at once.

**Layout** (popover 540 pt wide, ~725 pt tall; the scene is 384 pt, over half)
- Scene: no overlays except one chip bottom-left: the selection (name, position, keys) or,
  while the pointer is over the scene, how to select.
- Transport row: play/pause (filled when paused: the one thing to do), volume (the master
  group, moved out of the list), fly, head tracking, recenter; quit is a quiet power icon on the
  far right. Toggles that are on sit in a raised frame. Hovering a button writes its name and
  key into the status slot on the right, which otherwise shows where the head looks and the hour.
- Prompt: a framed field that starts with the "X_" of the menu bar mark (cursor blinks while
  focused). Send button appears when there is text; three dots while the model thinks. The
  line under it: an example (empty state), "Слушаю…", the model's answer with "⌘Z отменить",
  or an error with a "!" mark.
- Levers: water, wind, birds, rain, time and weather (flight joins first while flying); the
  rest behind "N more" with their pictograms. A lever is pictogram + slider (waveform fill up
  to the value) + chevron; name and value appear on hover. Generic: any new group the engine
  adds lands behind "more"; at least four levers are always shown.
- Drawer: the pictogram or chevron opens one group: back, title, the macro's value, then each
  parameter named with its slider and value (bright once the user set it; double-click
  returns it to the lever). Presets are a row of stops with the chosen name.
- Spectrogram: a 24 pt live strip; click to open (96 pt, scale, readout, hold/zoom as before).

**Scene shader** (`Stage/Shaders.swift`, `engrave`): the engraving is gone. Tone → grey per
material (near-black trees, dark ground, the river a little lighter, the herm grey stone),
exponential depth fog into a dusk-grey haze (by day) or near black (by night), a soft dark
contour only where objects overlap, a light vignette, an invisible dither against banding.
Sounds are faint pale rings (birds, river, rain, crickets); fire is a soft white light, no
colour. The selection is a pale outline plus corner marks. The violet point and beam are
mixed in rather than added, so they stay violet over any grey. Fewer texture reads than the
engraving (halo 8 taps instead of 16, beam 24 steps instead of 28).

**Keyboard**: the scene keeps the keyboard (arrows, ⇧↑↓, ⌥, tab, space); buttons hand it
back after a click. New: F fly, H head tracking, C recenter, return or / to the prompt, esc
in the prompt back to the scene, esc with nothing selected closes the drawer, ⌘← closes it
too, ⌘Q quits. Transport buttons are focusable and draw a 1.5 pt ring.

**Menu bar** (`tools/icon/make_menubar.py`): 25 × 18 pt (was 66 × 22). "ASMR" over "X_",
4 × 7 bitmap letters (M and X 5 × 7) on a 1 pt grid, italic by 1 pt stair-steps every 4
rows. Variants in `docs/ux/icon/menubar-variants.png`: (a) italic, short cursor — chosen;
(b) italic with a long cursor line — reads as an underline under X, less like a prompt;
(c) upright — clean, but loses the requested italic. Open state: the blue crest passes
A-S-M-R-X, the cursor stays blue and blinks.

**Not verified / open**
- CPU while open: `--bench` could not render (display asleep during the run: 0 fps). The
  audio engine alone measured ~9 % of a core here; rerun `--bench` with the screen on.
- Labels are English, the prompt area Russian (as before). One language for the whole
  popover is a product decision. (Done in section 4: Russian everywhere.)
- The lever sliders are not keyboard-focusable (the scene owns the arrows); VoiceOver can
  adjust them.

## 4. Second pass: sound vs world, Russian, the mark

Before: `docs/ux/after/` (section 3). After: `docs/ux/v2/` (static, selected, flight, water
drawer, all levers, night, mix drawer, and `without-levels` for a core without level controls).
The snapshots with a sound section were taken with `ASMR_PREVIEW_LEVELS=1` (inert 0 dB stand-ins,
see below) until the core's level controls are merged.

**Pictograms were invisible at rest** (`docs/ux/v2/icons-live-before.png` → `-after.png`, a live
AppKit render; snapshots draw icons through a Canvas and never showed the bug). SwiftUI configures
`IconLayerView` before it has a size, so its stroke layers were built later in `layout()` without
a stroke colour; only a hover re-ran `configure()` and coloured them. Layers are now coloured (and
given the screen's contents scale) when built. Animation still runs on hover only.

**Two sections instead of one list of sliders** (`Panel/Levers.swift`)
- **ЗВУК — громкость групп, дБ.** One trim per group (вода, ветер, листва, дождь, птицы,
  насекомые, костёр): controls with `ASMR_CONTROL_FLAG_LEVEL` (1 << 4), key `<group>.level`,
  −40…+12 dB, 0 default. Two columns of 22 pt rows: pictogram (click mutes = −40 dB, struck
  through; click again restores), name, rail with a tick at 0 dB, value ("−6 дБ", "выкл").
  Double-click the rail: 0 dB. Last cell: the mix's clarity ("Ясность"), which opens the mix
  drawer. A group's `*.gain` leaves its drawer once the group has a level. Without level
  controls the section keeps only the mix.
- **МИР — что происходит в лесу** (in flight: "полёт над лесом"). The macro levers, each with
  pictogram, name of what it physically sets ("Течение воды", "Скорость ветра", "Сила дождя",
  "Время суток", "Скорость полёта"…), the waveform slider, the value with units ("4.8 м/с",
  "0 мм/ч", "09:00", "×1.0") and the chevron to its drawer. Nothing is hover-only any more.
- Master volume stays in the transport.

**Transport**: flight is a two-segment switch "На земле | Полёт": the current mode is raised,
the other is what a click does (hint: "Взлететь над лесом F" / "Приземлиться F"). Head tracking
is a labelled toggle "Голова" with a new pictogram (a head with an AirPod in each ear and a turn
arc), struck through when off, a violet dot when on. The status shows the gaze as arrows
("← 35° ↑ 15°"); the hour moved to the world's "Время суток" lever when head tracking is on.
The scene is 336 pt (was 384) to make room; the popover is about 790 pt tall with every section.

**One language**: every label, hint, unit, preset name ("ручей по гальке", "тайга") and
accessibility label in the popover is Russian. Coppice Mono now has the full Russian Cyrillic
(`tools/font/coppice.py`, proof `docs/ux/v2/coppice-cyrillic.png`): А–я, Ёё in the same nib,
serifs and pixel grid; letters shared with Latin reuse its drawings, the small-capital lowercase
letters (в г д ж з и к л м н п т ц ч ш щ ъ ы ь э ю я) are one function of height. Also № « » …
− (minus) ⏎. No more fallback to a system face.

**Menu bar**: "ASMR" over "X3000" (was "X_"), 26 × 18 pt: the figures are 3 × 7 so the second
line is one point wider than "ASMR". The crest passes A S M R, then X 3 0 0 0; no cursor.
Variants in `docs/ux/icon/menubar-variants.png` (a, chosen; b, 4-wide figures, 30 pt, a little
easier to read; c, upright; the old X_). The app icon already reads ASMR / X / 3000.

**Not done / open**
- The "X_" at the start of the prompt field is drawn in `Prompt/PromptField.swift`
  (`PromptMark`), owned by the prompt work; it still says X_. Prompt help and accessibility
  strings there ("Send   Enter", "Listening", "Prompt") are still English.
- The gaze label is built in English in `Stage/Stage.swift` and translated at display time.
- Not checked in the running popover by eye (snapshots and a live AppKit icon render only).
