# Visual direction: "the wired" (after Serial Experiments Lain)

Subject: ASMR X3000 is a menu-bar app that puts a developer inside a spatial soundscape
(forest, underwater, space) through AirPods with head tracking. Audience: developers at a desk
in an office. Primary job of the onboarding: link the user's head to the sound world, then
point "forward" at the screen.

Why Lain: the series is about being connected by wires, with headphones and the hum of
power lines carrying a presence. That is literally this product: AirPods link your head to a
world that is somewhere else. We borrow its *visual grammar*, not its characters: humming
power lines, flat overexposed light against deep shadow, shadows filled with red halftone
dots, quiet lowercase title cards. No neon, no glow, no gradients, no ASCII-art terminal.

## Tokens

Colour (night, matching the dark panel):

| name    | hex       | role |
|---------|-----------|------|
| void    | `#000000` | background. True black, not a tinted near-black. |
| wire    | `#2A2B2F` | power lines, poles, hairlines, inactive UI |
| smog    | `#9EA39A` | secondary text, mid-tones of the halftone |
| glare   | `#E4E7DF` | overexposed highlights, primary text |
| blood   | `#A3262E` | the halftone shadow dots, and nothing else |
| listener| the app's existing violet | only the user: their head link, the lock, their own position |

Rule: red only ever appears as dots in shadows. Violet only ever means "you". Everything else
is the grey-green of a smoggy overexposed sky.

Type:
- **Voice**: Apple's serif (New York, `Font.system(.., design: .serif)`), lowercase, regular
  weight, generous tracking (+2–4%), at one size (22 pt) for the one line each screen says.
  Sentence fragments, plain words: "соединяем голову со звуком", "посмотри на экран".
  Never italic, never a highlighted word.
- **System**: Coppice Mono (already in the app) at 11 pt for state lines ("airpods — ждём
  сигнал", values), in smog. No log-prompt symbols, no spinner characters, no all-caps.

Layout (onboarding window, one composition reused for each step):

```
 ┌────────────────────────────────────────────────────────┐
 │  ╲____________╱‾‾‾‾‾‾‾‾‾‾‾‾╲______________╱  ┃         │  power lines sag across the
 │  ╲___________╱‾‾‾‾‾‾‾‾‾‾‾‾‾‾╲____________╱   ┃         │  top third; one pole at right
 │               ╲                       ╱       ┃         │  two wires drop down…
 │                ╲      ▒▒▓▓▓▒▒        ╱                  │
 │                 ╲   ▒▓██████▓▒      ╱                   │  …into the AirPods of a
 │                  ●▒▓███▓▓███▓▒●                         │  halftone head (dots, red in
 │                    ▒▓██▓▓▓██▓▒                          │  the shadow side, glare on lit)
 │                      ▒▓▓▓▓▓▒                            │
 │                                                         │
 │  соединяем голову со звуком                             │  serif voice line, left
 │  airpods — ждём сигнал                                  │  mono state line, left
 │                                                         │
 │  разрешить            без трекинга                      │  actions as text, left
 └────────────────────────────────────────────────────────┘
```

Everything left-aligned to one margin (40 pt). The head sits slightly right of centre, as
in the show's off-centre framing. No cards, no boxes, no rounded panels.

## The one memorable thing

The **halftone head on the wires**. It is rendered live (the existing distance-field bust
from AsciiArt.swift, shaded per cell) as an amplitude-modulated **dot screen**, not
characters. Dots are round, set on a 45°-rotated grid, and sized by luminance. On the lit side
they are glare dots on black. On the shadow side the dots turn blood red, as in the series.
Two power-line wires come down from the top and end in the AirPods.

- Waiting: the wires hum. They slowly vibrate a few px (a standing wave, sub-1 Hz) and the
  head turns by itself.
- Linked: a single pulse runs down both wires into the AirPods. The AirPods take the violet,
  the head eases over to the user's real pose, and from then on it mirrors them. This is
  the only orchestrated motion moment.
- Step 2, "посмотри на экран": the camera pushes in to an extreme close-up of one eye,
  framed like the series' eye shots, in halftone. The eye looks at you, then down to the
  screen centre. The target is a small blood-red cross. A thin wire-coloured ring closes
  as you hold still. On lock the iris takes the violet and the voice line changes to
  "вперёд — это экран".
- Step 3, screen: a side view in the same halftone language. The head in profile, a thin
  wire line to the screen drawn as a flat overexposed slab, the distance in mono. The size
  is adjusted with plain text − / +.
- Finish: "слушать" as the only action.

Texture: one very faint horizontal scan texture over the whole window (2–3% opacity), static.
Do not animate it, and do not add noise or chromatic aberration.

Reduce Motion: no wire hum. The pulse becomes an instant colour change. The head does not
self-turn.

## Self-review against the brief

- *Generic dark + single bright accent?* No. The base is smoggy grey-green tones on black,
  red is restricted to shadow dots, and violet keeps its existing meaning.
- *Terminal / ASCII?* Removed. Mono is kept only for small state lines, which the app
  already does.
- *Anime without cringe?* The anime comes from framing (off-centre, eye close-up,
  overexposure, red-dot shadows), not from drawn big-eyed characters, sparkles or glow.
- *Boldness in one place:* the halftone head and wires. Everything else is quiet text.

## App-wide (later, once the onboarding is approved)

- The 3D stage: shadows get the same red halftone dot screen as a post-process, and
  highlights clip toward glare. That one change ties every scene (forest, underwater,
  space) to the onboarding.
- The panel stays as it is apart from tokens (smog/glare text, wire hairlines).
- Clip objects on the stage get recognisable silhouettes, not grey spheres: a whale body,
  a dolphin, a comet with a tail, the current tunnel as rings.
