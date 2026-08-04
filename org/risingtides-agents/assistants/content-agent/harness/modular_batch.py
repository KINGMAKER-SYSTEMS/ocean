#!/usr/bin/env python3
"""Modular batch generator — compose rotated scenes (DB-backed) → generate → download all clips.

For a t2v-crop format (truck on MiniMax) it generates ONE wide side-shot per rotated scene
and downloads all 5 crops (front/cab/center/bed/tailgate) — 5 distinct clips per ~$0.28
generation. Variety comes from rotating N scenes (law 0); the 5-way crop multiplies each
scene cheaply. Scenes come from `prompt_library.py compose` (the canon in SQLite).

Env: CONTENT_LAB_API_URL (+ CONTENT_LAB_API_KEY if the deploy requires it).
Usage: modular_batch.py <format> <num_scenes> [out_dir]
"""
import json, os, subprocess, sys, time
import requests

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.environ.get("CONTENT_LAB_API_URL", "https://risingtides-content-lab-production.up.railway.app").rstrip("/")
KEY = os.environ.get("CONTENT_LAB_API_KEY")
HEADERS = {"Authorization": f"Bearer {KEY}"} if KEY else {}

fmt = sys.argv[1]
scenes = int(sys.argv[2]) if len(sys.argv) > 2 else 10
out = sys.argv[3] if len(sys.argv) > 3 else os.path.join(HERE, "..", "output", f"modular-{fmt}")
os.makedirs(out, exist_ok=True)
project = f"modular-{fmt}"
requests.post(f"{BASE}/api/projects/", json={"name": project}, headers=HEADERS, timeout=30)

recs = json.loads(subprocess.run(
    ["python3", os.path.join(HERE, "prompt_library.py"), "compose", fmt, "--n", str(scenes), "--json"],
    capture_output=True, text=True).stdout)

total = 0
for si, r in enumerate(recs, 1):
    if r["method"] != "t2v":
        sys.exit(f"modular_batch supports t2v-crop formats; '{fmt}' is {r['method']} (i2v needs anchor-gen)")
    data = {"prompt": r["prompt"], "provider": r["provider"], "count": "1", "duration": "6",
            "aspect_ratio": "9:16", "resolution": "768p", "project": project}
    for k, v in (r.get("extra") or {}).items():
        data[k] = str(v).lower() if isinstance(v, bool) else str(v)
    jid = requests.post(f"{BASE}/api/video/generate", data=data, headers=HEADERS, timeout=120).json()["job_id"]
    print(f"scene {si}/{scenes}: {jid}", flush=True)
    while True:
        j = requests.get(f"{BASE}/api/video/jobs/{jid}", headers=HEADERS, timeout=60).json()
        vids = j.get("videos", [])
        if vids and all(x.get("status") in ("done", "error") for x in vids):
            break
        time.sleep(12)
    ci = 0
    for vid in vids:
        if vid.get("status") != "done":
            continue
        for cr in (vid.get("crops") or ([{"url": vid.get("url")}] if vid.get("url") else [])):
            if not cr.get("url"):
                continue
            raw = requests.get(BASE + cr["url"], headers=HEADERS, timeout=180).content
            ci += 1
            total += 1
            open(os.path.join(out, f"{fmt}_s{si:02d}_c{ci}.mp4"), "wb").write(raw)
    print(f"  scene {si}: {ci} clips (total {total})", flush=True)

print(f"\nDONE: {total} clips in {out}/", flush=True)
