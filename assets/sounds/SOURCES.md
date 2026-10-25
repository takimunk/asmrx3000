# Sound files

The shipped app uses no sound files. All three scenes (Forest, Underwater, Body scan) are
procedural: every sound is synthesised by the physical models in `core/`.

The clip engine (`ClipBank` in `core/src/clips.rs`) can still play WAV files from this folder
for experiments. Put 16-bit mono WAVs here and reference them from a scene JSON by relative
path; `scripts/build-macos.sh` copies any `.wav` a shipped scene references into the bundle.
The folder is otherwise empty, and nothing here is committed besides this note.

If you add recordings, record their origin and license here (URL, author, license, required
attribution). Prefer CC0, public domain or CC BY. The earlier list of field recordings used for
comparison is in git history.
