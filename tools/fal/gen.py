#!/usr/bin/env python3
"""fal.ai text-to-audio stem generator (stdlib only). See README.md."""
import argparse, json, os, subprocess, sys, time, urllib.request, urllib.error
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
QUEUE = "https://queue.fal.run/"
LOG = HERE / "spend.log"

# short name -> (endpoint, max_seconds, usd_per_second, usd_flat, builder)
def _eleven(p, d): return {"text": p["prompt"], "duration_seconds": d,
                           "prompt_influence": p.get("prompt_influence", 0.3), "loop": bool(p.get("loop", False))}
def _stable(p, d): return {"prompt": p["prompt"], "seconds_total": int(d)}
def _mm(p, d): return {"prompt": p["prompt"], "duration": float(d), "num_steps": 25, "cfg_strength": 4.5,
                       "negative_prompt": "music, speech, voice"}
def _cass(p, d): return {"prompt": p["prompt"], "duration": int(d)}
MODELS = {
    "el2":    ("fal-ai/elevenlabs/sound-effects/v2", 22, 0.002, 0.0, _eleven),
    "sa25":   ("fal-ai/stable-audio-25/text-to-audio", 190, 0.0, 0.20, _stable),
    "mmaudio":("fal-ai/mmaudio-v2/text-to-audio", 30, 0.001, 0.0, _mm),
    "cassette":("cassetteai/sound-effects-generator", 30, 0.0, 0.01, _cass),
}

def get_key():
    k = os.environ.get("FAL_API_KEY", "").strip() or os.environ.get("FAL_KEY", "").strip()
    if k: return k
    for p in (ROOT / ".env", ROOT.parent.parent.parent / ".env"):   # worktree -> main checkout
        if p.is_file():
            for line in p.read_text().splitlines():
                if line.startswith("FAL_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"\'')
    return None

def req(url, key, body=None):
    r = urllib.request.Request(url, data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": "Key " + key, "Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=120) as f:
        return json.load(f)

def run(endpoint, key, body):
    sub = req(QUEUE + endpoint, key, body)
    st, rs = sub["status_url"], sub["response_url"]
    for _ in range(300):
        s = req(st, key)
        if s.get("status") == "COMPLETED": return req(rs, key)
        if s.get("status") in ("FAILED", "ERROR"): raise RuntimeError(str(s)[:300])
        time.sleep(2)
    raise TimeoutError("queue timeout")

def find_url(o):
    if isinstance(o, str) and o.startswith("http"): return o
    if isinstance(o, dict):
        for k in ("audio", "audio_file", "url"):
            if k in o:
                u = find_url(o[k])
                if u: return u
    return None

def cost(m, d):
    _, _, ps, flat, _ = MODELS[m]
    return flat + ps * d

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prompts", default=str(HERE / "prompts.json"))
    ap.add_argument("--out", default=str(ROOT / "assets/sounds/generated"))
    ap.add_argument("--dl", default=None, help="download dir for raw files (must be new/empty-ish)")
    ap.add_argument("--model", default="el2", choices=list(MODELS), help="default model; per-prompt `model` overrides")
    ap.add_argument("--scene", action="append"); ap.add_argument("--name", action="append")
    ap.add_argument("--variations", type=int, help="override variations (e.g. 1)")
    ap.add_argument("--budget", type=float, default=8.0, help="abort if logged+planned spend exceeds this ($)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    logged = 0.0
    if LOG.exists():
        for l in LOG.read_text().splitlines():
            try: logged += float(l.rsplit("\t", 1)[1].lstrip("$"))
            except Exception: pass
    jobs = []
    for it in json.load(open(a.prompts)):
        if a.scene and it["scene"] not in a.scene: continue
        if a.name and it["name"] not in a.name: continue
        m = it.get("model", a.model)
        d = min(it["duration"], MODELS[m][1])
        for n in range(1, (a.variations or int(it.get("variations", 1))) + 1):
            jobs.append((it, m, d, n, Path(a.out) / it["scene"] / f'{it["name"]}_{m}_{n}.wav'))
    todo = [j for j in jobs if not j[4].exists()]
    est = sum(cost(m, d) for _, m, d, _, _ in todo)
    for it, m, d, n, dst in jobs:
        print(("SKIP " if dst.exists() else "GEN  ") + f"{dst.relative_to(ROOT) if dst.is_relative_to(ROOT) else dst} {d}s ${cost(m, d):.3f}")
    print(f"\n{len(todo)} generation(s), est ${est:.2f}; already logged ${logged:.2f}; budget ${a.budget:.2f}")
    if a.dry_run: return
    if logged + est > a.budget: sys.exit("over budget; narrow selection or raise --budget")
    key = get_key()
    if not key: sys.exit("No FAL_API_KEY in env or .env")
    dl = Path(a.dl or HERE / "dl"); dl.mkdir(parents=True, exist_ok=True)
    errs = 0
    for it, m, d, n, dst in todo:
        ep, _, _, _, build = MODELS[m]
        try:
            out = run(ep, key, build(it, d))
            url = find_url(out)
            if not url: raise RuntimeError("no audio url in response")
            ext = os.path.splitext(url.split("?")[0])[1] or ".bin"
            raw = dl / f'{it["scene"]}_{it["name"]}_{m}_{n}{ext}'
            with urllib.request.urlopen(url, timeout=120) as f: raw.write_bytes(f.read())
            dst.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@48000", "-c", "1", str(raw), str(dst)], check=True)
        except Exception as e:
            errs += 1
            msg = e.read()[:200].decode(errors="replace") if isinstance(e, urllib.error.HTTPError) else e
            print(f"  ERROR {dst.name}: {type(e).__name__} {msg}", file=sys.stderr)
            if errs >= 3: sys.exit("3 errors, stopping")
            continue
        c = cost(m, d)
        with open(LOG, "a") as f:
            f.write(f"{datetime.now():%Y-%m-%d %H:%M:%S}\t{m}\t{dst.name}\t{d}s\t${c:.4f}\n")
        print(f"  saved {dst.name} (${c:.3f})")

if __name__ == "__main__":
    main()
