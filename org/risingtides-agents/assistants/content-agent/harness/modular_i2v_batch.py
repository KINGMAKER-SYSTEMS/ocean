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
import base64, io, json, os, subprocess, sys, time
from pathlib import Path

import requests
from PIL import Image

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


def _bg_is_black(path: Path) -> bool:
    """True if the 4 corners are dark — rejects gemini's occasional white-bg glitch
    (the black-crush only fixes dark->black, not white)."""
    im = Image.open(path).convert("L")
    W, H = im.size
    s = 30
    corners = [(0, 0), (W - s, 0), (0, H - s), (W - s, H - s)]
    for cx, cy in corners:
        crop = im.crop((cx, cy, cx + s, cy + s))
        if sum(crop.getdata()) / (s * s) > 45:      # a corner is bright => not a black bg
            return False
    return True


def mint_anchor(prompt: str, path: Path, tries: int = 3):
    """Mint a fresh on-brand anchor with gemini-3-pro-image (9:16). Retries if the background
    comes back non-black (a known gemini glitch that the crush can't fix)."""
    if not GEMINI_KEY:
        sys.exit("GEMINI_API_KEY not set — needed to mint anchors")
    body = {"contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"responseModalities": ["IMAGE"], "imageConfig": {"aspectRatio": "9:16"}}}
    for attempt in range(tries):
        r = requests.post(f"https://generativelanguage.googleapis.com/v1beta/models/{GEMINI_MODEL}:generateContent",
                          headers={"x-goog-api-key": GEMINI_KEY, "Content-Type": "application/json"},
                          json=body, timeout=180)
        r.raise_for_status()
        blob = next((p.get("inlineData") or p.get("inline_data")
                     for p in r.json()["candidates"][0]["content"]["parts"]
                     if p.get("inlineData") or p.get("inline_data")), None)
        if not blob:
            raise RuntimeError(f"no image in gemini response: {json.dumps(r.json())[:200]}")
        path.write_bytes(base64.b64decode(blob["data"]))
        if _bg_is_black(path) or attempt == tries - 1:
            return path
    return path


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


# Enforces Eric's spec on every clip: pure #000000 backdrop (crush dark bg to true black),
# saucer edge-to-edge, cup dead-center, AND trim any end-of-clip intruder (wan sometimes
# hallucinates a hand/pitcher creeping in near the end). Prompt guards don't stop these.
CRUSH = "curves=all='0/0 0.14/0 1/1'"


def _gray_frame(path: Path, t: float, w: int = 200):
    raw = subprocess.run(["ffmpeg", "-v", "quiet", "-ss", f"{t}", "-i", str(path), "-frames:v", "1",
                          "-vf", f"{CRUSH},scale={w}:-2", "-f", "image2pipe", "-vcodec", "png", "-"],
                         capture_output=True).stdout
    return Image.open(io.BytesIO(raw)).convert("L") if raw else None


def _clean_end(path: Path, dur: float):
    """Return the timestamp to trim to if an intruder enters (bright pixels appear OUTSIDE the
    stable cup, against the pure-black bg), else None. Cup+bg must be crushed first."""
    base = _gray_frame(path, min(0.5, dur / 3))
    if base is None:
        return None
    W, H = base.size
    bb = base.point(lambda p: 255 if p > 60 else 0).getbbox()
    if not bb:
        return None
    x0, y0, x1, y1 = bb
    mx, my = int(W * .03), int(H * .03)
    dx0, dy0, dx1, dy1 = x0 - mx, y0 - my, x1 + mx, y1 + my
    t = 1.0
    while t < dur:
        f = _gray_frame(path, t)
        if f is None:
            break
        px = f.point(lambda p: 255 if p > 60 else 0).load()
        out = sum(1 for yy in range(0, H, 2) for xx in range(0, W, 2)
                  if px[xx, yy] and not (dx0 <= xx <= dx1 and dy0 <= yy <= dy1))
        if out > 12:                      # an intruder appeared outside the cup
            return max(round(t - 0.3, 2), 2.0)
        t += 0.1
    return None


def frame_clip(path: Path):
    """Trim any end intruder, crush bg to pure black, crop to the cup, scale saucer to full
    width, pad to 9:16 dead-center. No-op if the cup bbox can't be found."""
    dur = float(subprocess.run(["ffprobe", "-v", "quiet", "-show_entries", "format=duration",
                                "-of", "csv=p=0", str(path)], capture_output=True, text=True).stdout or 0)
    ce = _clean_end(path, dur) if dur else None
    raw = subprocess.run(["ffmpeg", "-v", "quiet", "-ss", "0.1", "-i", str(path),
                          "-frames:v", "1", "-vf", CRUSH, "-f", "image2pipe", "-vcodec", "png", "-"],
                         capture_output=True).stdout
    im = Image.open(io.BytesIO(raw)).convert("L")
    W, H = im.size
    bb = im.point(lambda p: 255 if p > 22 else 0).getbbox()
    if not bb:
        return
    x0, y0, x1, y1 = bb
    bw, bh = x1 - x0, y1 - y0
    sh = round(bh * W / bw / 2) * 2
    if sh > H:
        vf = f"{CRUSH},crop={bw}:{bh}:{x0}:{y0},scale=-2:{H},pad={W}:{H}:({W}-iw)/2:0:black"
    else:
        vf = f"{CRUSH},crop={bw}:{bh}:{x0}:{y0},scale={W}:{sh},pad={W}:{H}:0:({H}-ih)/2:black"
    # min length 5s: if trimming an intruder drops the clip under 5s, slow it just enough to
    # push it back over 5 (~10% nudge, imperceptible on a gentle loop).
    eff = ce if ce else dur
    if eff and eff < 5.0:
        vf += f",setpts={5.1 / eff:.4f}*PTS"
    tmp = path.with_suffix(".framed.mp4")
    cmd = ["ffmpeg", "-y", "-v", "quiet"]
    if ce:
        cmd += ["-t", str(ce)]            # input trim (before -i) so setpts can then re-stretch
    cmd += ["-i", str(path), "-vf", vf, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", str(tmp)]
    subprocess.run(cmd, check=True)
    tmp.replace(path)


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
            clip = out / f"{fmt}_s{si:02d}_c{ci}.mp4"
            clip.write_bytes(raw)
            try:
                frame_clip(clip)   # pure-black bg + saucer edge-to-edge + dead-center
            except Exception as e:
                print(f"  frame_clip warn {clip.name}: {e}", flush=True)
            total += 1
        print(f"  scene {si}: {len(urls)} clips (total {total})", flush=True)
    print(f"\nDONE: {total} clips in {out}/ , anchor bank in {bank}/", flush=True)


if __name__ == "__main__":
    main()
