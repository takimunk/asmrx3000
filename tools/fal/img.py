#!/usr/bin/env python3
"""fal.ai image generation for the onboarding art (stdlib only; reuses gen.py's HTTP helpers).

  img.py t2i  --prompt "..." --out DIR [--n 4] [--seed S]       fal-ai/flux/schnell
  img.py edit --image URL_OR_FILE --prompt "..." --out FILE      fal-ai/flux-pro/kontext

Every call is logged to spend.log; --budget caps this tool's own logged spend (rows tagged img:).
"""
import argparse, base64, json, sys, urllib.request
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from gen import LOG, get_key, run  # noqa: E402

PRICES = {"fal-ai/flux/schnell": 0.003, "fal-ai/flux-pro/kontext": 0.04}

def spent():
    total = 0.0
    if LOG.exists():
        for l in LOG.read_text().splitlines():
            parts = l.split("\t")
            if len(parts) >= 2 and parts[1].startswith("img:"):
                try: total += float(parts[-1].lstrip("$"))
                except ValueError: pass
    return total

def log(model, name, usd):
    with open(LOG, "a") as f:
        f.write(f"{datetime.now():%Y-%m-%d %H:%M:%S}\timg:{model}\t{name}\t-\t${usd:.4f}\n")

def images(out):
    return [i["url"] for i in out.get("images", []) if isinstance(i, dict) and "url" in i]

def save(url, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=120) as f: dst.write_bytes(f.read())

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["t2i", "edit"])
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--image")
    ap.add_argument("--n", type=int, default=1)
    ap.add_argument("--seed", type=int)
    ap.add_argument("--size", default="square")
    ap.add_argument("--budget", type=float, default=2.0)
    a = ap.parse_args()
    model = "fal-ai/flux/schnell" if a.mode == "t2i" else "fal-ai/flux-pro/kontext"
    est = PRICES[model] * a.n
    if spent() + est > a.budget: sys.exit(f"over budget: spent ${spent():.3f} + ${est:.3f} > ${a.budget:.2f}")
    key = get_key()
    if not key: sys.exit("No FAL_API_KEY")
    if a.mode == "t2i":
        body = {"prompt": a.prompt, "image_size": a.size, "num_images": a.n, "num_inference_steps": 4,
                "enable_safety_checker": True}
        if a.seed is not None: body["seed"] = a.seed
    else:
        img = a.image
        if not img.startswith("http"):
            img = "data:image/png;base64," + base64.b64encode(Path(img).read_bytes()).decode()
        body = {"prompt": a.prompt, "image_url": img, "num_images": 1, "output_format": "png"}
        if a.seed is not None: body["seed"] = a.seed
    out = run(model, key, body)
    urls = images(out)
    dst = Path(a.out)
    for i, u in enumerate(urls):
        p = dst / f"{i}.png" if a.mode == "t2i" else dst
        save(u, p)
        print(p, u)
    log(model.split("/")[-1], dst.name, PRICES[model] * max(1, len(urls)))
    print(f"spent so far (img): ${spent():.3f}")

if __name__ == "__main__":
    main()
