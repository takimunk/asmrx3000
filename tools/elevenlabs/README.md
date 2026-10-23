# ElevenLabs stem generator

Generates atomic ambient stems with the ElevenLabs Sound Effects API
(`POST /v1/sound-generation`, model `eleven_text_to_sound_v2`) and saves them as
48 kHz mono 16-bit WAV under `assets/sounds/generated/<scene>/<name>_<n>.wav`.

## Key
Either `export ELEVENLABS_API_KEY=...` or put the key alone in
`~/.config/asmrx3000/elevenlabs_key` (chmod 600). The key is never printed.

## Usage
    python3 tools/elevenlabs/gen.py --dry-run              # list requests + estimated cost
    python3 tools/elevenlabs/gen.py --scene space           # one scene
    python3 tools/elevenlabs/gen.py --name humpback_song    # one prompt
    python3 tools/elevenlabs/gen.py                         # everything (paid)

Options: `--prompts FILE`, `--out DIR`, `--format` (default `mp3_44100_128`;
`pcm_48000` is accepted if your plan allows), `--usd-per-1k` (price assumption).
Existing files are skipped, so re-runs are free. After each call the script
reports the change in `character_count` from `/v1/user/subscription` as the
credits used (best effort).

## prompts.json fields
`scene`, `name`, `prompt`, `duration` (0.5-30 s), `loop` (seamless loop, only
v2), `prompt_influence` (0-1, higher = literal, less varied), `variations`
(number of separate generations; each differs).

## Limits and notes
- Max 30 s per request. Longer stems: generate several variations and
  crossfade/shuffle them in the engine, or use `loop: true` clips.
- Cost: 40 credits/s with explicit duration (~$0.26 per 30 s at Creator rates).
- Output is stereo-ish MP3; converted to mono with `afconvert`. Listen to each
  take: the model sometimes adds music or extra sources despite the prompt.
- Free plan output requires attribution to elevenlabs.io; use a paid plan for
  commercial release.
