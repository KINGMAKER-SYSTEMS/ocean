#!/usr/bin/env python3
"""Modular i2v batch — the anchor-bank path (coffee etc.).

For an i2v format, variety comes from the ANCHOR, not prompt slots. This:
  1. composes N rotated anchor prompts (prompt_library.py compose)
  2. mints each anchor with gemini-3-pro-image (a fresh on-brand hero frame)
  3. i2v-animates each with the format's FIXED motion_prompt on the content lab
  4. downloads clips, distributing a batch across the bank (law 0: never one anchor N times)

The mug/crema/framing changes every scene; the crowned look + motion recipe do not.

Env: GEMINI_API_KEY (anchor minting); CONTENT_LAB_API_URL (+ CONTENT_LAB_API_KEY if required).
Usage: modular_i2v_batch.py <format> <num_scenes> [clips_per_anchor] [out_dir]
"""
import base64, json, os, subprocess, sys, time
from pathlib import Path

import requests

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
BASE = os.environ.get("CONTENT_LAB_API_URL", "https://risingtides-content-lab-production.up.railway.app").rstrip("/")
KEY = os.environ.get("CONTENT_LAB_API_KEY")
HDR = {"Authorization": f"Bearer {KEY}"} if KEY else {}
GEMINI_KEY = os.environ.get("GEMINI_API_KEY", "").strip()
GEMINI_MODEL = "gemini-3-pro-image"

fmt = sys.argv[1] if len(sys.argv) > 1 else "coffee-tok"
n_scenes = int(sys.argv[2]) if len(sys.argv) > 2 else 6
clips_per = int(sys.argv[3]) if len(sys.argv) > 3 else 1
out = Path(sys.argv[4]) if len(sys.argv) > 4 else ROOT / "output" / f"modular-{fmt}"
bank = ROOT / "anchors" / f"{fmt}-bank"
out.mkdir(parents=True, exist_ok=True)
bank.mkdir(parents=True, exist_ok=True)
project = f"modular-{fmt}"


def mint_anchor(prompt: str, path: Path):
    """Mint a fresh on-brand anchor with gemini-3-pro-image (9:16)."""
    if not GEMINI_KEY:
        sys.exit("GEMINI_API_KEY not set — needed to mint anchors")
    body = {"contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"responseModalities": ["IMAGE"], "imageConfig": {"aspectRatio": "9:16"}}}
    r = requests.post(f"https://generativelanguage.googleapis.com/v1beta/models/{GEMINI_MODEL}:generateContent",
                      headers={"x-goog-api-key": GEMINI_KEY, "Content-Type": "application/json"},
                      json=body, timeout=180)
    r.raise_for_status()
    for p in r.json()["candidates"][0]["content"]["parts"]:
        blob = p.get("inlineData") or p.get("inline_data")
        if blob:
            path.write_bytes(base64.b64decode(blob["data"]))
            return path
    raise RuntimeError(f"no image in gemini response: {json.dumps(r.json())[:200]}")


def i2v(anchor: Path, motion_prompt: str, provider: str, count: int) -> list[str]:
    """Submit an i2v job on the content lab, poll, return clip URLs."""
    data = {"prompt": motion_prompt, "provider": provider, "count": str(count),
            "duration": "6", "aspect_ratio": "9:16", "resolution": "720p", "project": project}
    files = {"media": ("anchor.png", anchor.read_bytes(), "image/png")}
    jid = requests.post(f"{BASE}/api/video/generate", data=data, files=files, headers=HDR, timeout=120).json()["job_id"]
    while True:
        j = requests.get(f"{BASE}/api/video/jobs/{jid}", headers=HDR, timeout=60).json()
        vids = j.get("videos", [])
        if vids and all(v.get("status") in ("done", "error") for v in vids):
            break
        time.sleep(10)
    urls = []
    for v in vids:
        if v.get("status") == "done":
            urls += [c["url"] for c in (v.get("crops") or [])] or ([v["url"]] if v.get("url") else [])
    return urls


def main():
    requests.post(f"{BASE}/api/projects/", json={"name": project}, headers=HDR, timeout=30)
    recs = json.loads(subprocess.run(
        ["python3", str(HERE / "prompt_library.py"), "compose", fmt, "--n", str(n_scenes), "--json"],
        capture_output=True, text=True).stdout)
    total = 0
    for si, r in enumerate(recs, 1):
        if r["method"] != "i2v":
            sys.exit(f"modular_i2v_batch is for i2v formats; '{fmt}' is {r['method']}")
        tag = "_".join(v.split()[1] for v in r["combo"].values() if v)[:24]
        anchor = bank / f"{fmt}_s{si:02d}.png"
        try:
            mint_anchor(r["anchor_prompt"], anchor)
            print(f"scene {si}/{n_scenes}: minted anchor {anchor.name} [{tag}]", flush=True)
        except Exception as e:
            print(f"scene {si}: anchor mint FAILED: {e}", flush=True)
            continue
        try:
            urls = i2v(anchor, r["motion_prompt"], r["provider"], clips_per)
        except Exception as e:
            print(f"scene {si}: i2v FAILED: {e}", flush=True)
            continue
        for ci, u in enumerate(urls, 1):
            raw = requests.get(BASE + u, headers=HDR, timeout=180).content
            (out / f"{fmt}_s{si:02d}_c{ci}.mp4").write_bytes(raw)
            total += 1
        print(f"  scene {si}: {len(urls)} clips (total {total})", flush=True)
    print(f"\nDONE: {total} clips in {out}/ , anchor bank in {bank}/", flush=True)


if __name__ == "__main__":
    main()
