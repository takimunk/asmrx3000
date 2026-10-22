#!/usr/bin/env python3
"""ElevenLabs sound-effect stem generator (stdlib only). See README.md."""
import argparse, json, os, subprocess, sys, urllib.request, urllib.error
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
API = "https://api.elevenlabs.io/v1"
CREDITS_PER_SEC = 40          # documented: 40 credits/s when duration is given
USD_PER_1K_CREDITS = 0.22     # ~Creator plan ($22 / 100k); adjust with --usd-per-1k

def get_key():
    k = os.environ.get("ELEVENLABS_API_KEY", "").strip()
    if k:
        return k
    p = Path.home() / ".config/asmrx3000/elevenlabs_key"
    if p.is_file():
        return p.read_text().strip()
    return None

def http(method, path, key, body=None):
    req = urllib.request.Request(API + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"xi-api-key": key, "Content-Type": "application/json", "Accept": "*/*"})
    return urllib.request.urlopen(req, timeout=180)

def used_credits(key):
    try:
        with http("GET", "/user/subscription", key) as r:
            return json.load(r).get("character_count")
    except Exception:
        return None

def to_wav(src, dst):
    subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@48000", "-c", "1", str(src), str(dst)], check=True)

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prompts", default=str(HERE / "prompts.json"))
    ap.add_argument("--out", default=str(ROOT / "assets/sounds/generated"))
    ap.add_argument("--scene", action="append", help="only this scene (repeatable)")
    ap.add_argument("--name", action="append", help="only this prompt name (repeatable)")
    ap.add_argument("--format", default="mp3_44100_128", help="ElevenLabs output_format (default mp3_44100_128; pcm_48000 may need a higher plan)")
    ap.add_argument("--usd-per-1k", type=float, default=USD_PER_1K_CREDITS)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    items = json.load(open(a.prompts))
    jobs = []
    for it in items:
        if a.scene and it["scene"] not in a.scene: continue
        if a.name and it["name"] not in a.name: continue
        for n in range(1, int(it.get("variations", 1)) + 1):
            dst = Path(a.out) / it["scene"] / f'{it["name"]}_{n}.wav'
            jobs.append((it, n, dst))

    total_c, todo = 0, 0
    key = None if a.dry_run else get_key()
    if not a.dry_run and not key:
        sys.exit("No API key: set ELEVENLABS_API_KEY or write ~/.config/asmrx3000/elevenlabs_key")

    for it, n, dst in jobs:
        credits = it["duration"] * CREDITS_PER_SEC
        exists = dst.exists()
        tag = "SKIP (exists)" if exists else "GEN "
        print(f'{tag:14} {dst.relative_to(ROOT) if dst.is_relative_to(ROOT) else dst}  {it["duration"]}s loop={it.get("loop", False)} infl={it.get("prompt_influence", 0.3)} ~{credits:.0f}cr')
        if a.dry_run:
            if not exists:
                print(f'               prompt: {it["prompt"]}')
        if exists: continue
        todo += 1; total_c += credits
        if a.dry_run: continue

        body = {"text": it["prompt"], "duration_seconds": it["duration"],
                "prompt_influence": it.get("prompt_influence", 0.3),
                "loop": bool(it.get("loop", False)), "model_id": "eleven_text_to_sound_v2"}
        before = used_credits(key)
        try:
            with http("POST", f"/sound-generation?output_format={a.format}", key, body) as r:
                data = r.read()
                hdrs = {k: v for k, v in r.headers.items() if "cost" in k.lower() or "credit" in k.lower()}
        except urllib.error.HTTPError as e:
            print(f"  ERROR {e.code}: {e.read()[:300].decode(errors='replace')}", file=sys.stderr)
            if e.code in (401, 403, 429): sys.exit("stopping")
            continue
        except Exception as e:
            print(f"  ERROR {type(e).__name__}: {e}", file=sys.stderr); continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        ext = "pcm" if a.format.startswith("pcm") else ("opus" if a.format.startswith("opus") else "mp3")
        if ext == "pcm":
            rate = a.format.split("_")[1]
            import wave
            with wave.open(str(dst), "wb") as w:
                w.setnchannels(1); w.setsampwidth(2); w.setframerate(int(rate)); w.writeframes(data)
            if rate != "48000":
                tmp = dst.with_suffix(".tmp.wav"); dst.rename(tmp); to_wav(tmp, dst); tmp.unlink()
        else:
            tmp = dst.with_suffix("." + ext); tmp.write_bytes(data)
            to_wav(tmp, dst); tmp.unlink()
        after = used_credits(key)
        extra = f" headers={hdrs}" if hdrs else ""
        if before is not None and after is not None:
            extra += f" credits_used={after - before}"
        print(f"  saved {dst.name} ({len(data)//1024} KiB){extra}")

    print(f"\n{todo} generation(s) needed, ~{total_c:.0f} credits, ~${total_c/1000*a.usd_per_1k:.2f} "
          f"(assumes {CREDITS_PER_SEC} cr/s, ${a.usd_per_1k}/1k credits)")

if __name__ == "__main__":
    main()
