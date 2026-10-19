#!/usr/bin/env python3
"""Build an unsigned, revision-addressed desktop candidate; never install it."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

SURFACE = Path(__file__).resolve().parents[1]
ROOT = SURFACE.parents[1]
RUNTIME = ROOT / "platform/ocean-os"
NATIVE = SURFACE / "crates/ocean-tauri"
TARGETS = {
    "aarch64-apple-darwin": ("Darwin", "arm64"),
    "x86_64-apple-darwin": ("Darwin", "x86_64"),
    "x86_64-pc-windows-msvc": ("Windows", "amd64"),
}
TAURI_VERSION = "2.11.3"
ABOUT_VERSION = "0.9.1"
TAURI_HASHES = {
    "aarch64-apple-darwin": "69716bda20d50e9d3bbbe9797fe303dd55aef903c1aba19731b7355d4fd546b0",
    "x86_64-apple-darwin": "9bde55fe15646f1c814eed9ee25da52a79d2c0371ad75ccdb9abf74e93e64069",
    "x86_64-pc-windows-msvc": "a172fa09b791fa885739d252e0ba8b29d0b426fc0c4e3414158308d2da8976f8",
}
ABOUT_HASHES = {
    "aarch64-apple-darwin": "6a38fe166d17a674269d4373256c0b6bd93acc2553e12de0517cb9ecc73c9c02",
    "x86_64-pc-windows-msvc": "318893aff6b9efd60f70470f5827b9577ae20a805cf9732d5612862a78508581",
}
LEGAL_FILES = ("LICENSE", "LICENSE-APACHE", "LICENSE-MIT", "NOTICE.md", "CREDITS.md", "TRADEMARKS.md")


def build_env():
    env = {k: v for k, v in os.environ.items() if not k.startswith(("OCEAN_", "TAURI_", "TRUNK_", "APPLE_"))}
    for key in ("RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR", "NO_COLOR"):
        env.pop(key, None)
    env["CARGO_TERM_COLOR"] = "never"
    return env


def run(args, cwd=ROOT, capture=False):
    print("+", " ".join(map(str, args)), flush=True)
    result = subprocess.run(list(map(str, args)), cwd=cwd, env=build_env(), check=True,
                            stdout=subprocess.PIPE if capture else None, text=True, encoding="utf-8")
    return result.stdout.strip() if capture else None


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def clean_revision():
    if run(["git", "status", "--porcelain", "--untracked-files=normal"], capture=True):
        raise RuntimeError("Desktop packaging requires a clean committed checkout")
    remote = run(["git", "remote", "get-url", "origin"], capture=True).removesuffix(".git").lower()
    if remote not in ("https://github.com/kingmaker-systems/ocean", "git@github.com:kingmaker-systems/ocean"):
        raise RuntimeError("Desktop packaging requires the canonical organization repository")
    return run(["git", "rev-parse", "HEAD"], capture=True)


def unpack_tool(payload, name, zipped):
    """Extract exactly one regular named binary, never archive-supplied paths."""
    if zipped:
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            found = [item for item in archive.infolist() if Path(item.filename).name == name and not item.is_dir()]
            if len(found) != 1 or (found[0].external_attr >> 16) & 0o170000 == 0o120000:
                raise RuntimeError("Tool archive lacks one regular executable")
            return archive.read(found[0])
    with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
        found = [item for item in archive.getmembers() if Path(item.name).name == name and item.isfile()]
        if len(found) != 1:
            raise RuntimeError("Tool archive lacks one regular executable")
        with archive.extractfile(found[0]) as file:
            return file.read()


def download_tool(url, expected, directory, name, zipped):
    request = urllib.request.Request(url, headers={"User-Agent": "Ocean-desktop-build"})
    with urllib.request.urlopen(request, timeout=120) as response:
        payload = response.read(128 * 1024 * 1024 + 1)
    if len(payload) > 128 * 1024 * 1024 or hashlib.sha256(payload).hexdigest() != expected:
        raise RuntimeError("Tool archive checksum mismatch")
    directory.mkdir(parents=True, exist_ok=True)
    destination = directory / name
    destination.write_bytes(unpack_tool(payload, name, zipped))
    destination.chmod(0o755)
    return destination


def tools(target):
    directory = SURFACE / "target/desktop-tools" / target
    suffix = ".exe" if "windows" in target else ""
    tauri = download_tool(
        f"https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v{TAURI_VERSION}/cargo-tauri-{target}.zip",
        TAURI_HASHES[target], directory, "cargo-tauri" + suffix, True)
    if target in ABOUT_HASHES:
        about = download_tool(
            f"https://github.com/EmbarkStudios/cargo-about/releases/download/{ABOUT_VERSION}/cargo-about-{ABOUT_VERSION}-{target}.tar.gz",
            ABOUT_HASHES[target], directory, "cargo-about" + suffix, False)
    else:
        # Upstream 0.9.1 has no Intel Mac asset; build its locked release source.
        run(["cargo", "install", "cargo-about", "--version", ABOUT_VERSION, "--locked", "--features", "cli", "--root", directory], cwd=SURFACE)
        about = directory / "bin/cargo-about"
    if run([tauri, "--version"], capture=True) != f"tauri-cli {TAURI_VERSION}":
        raise RuntimeError("Unexpected Tauri CLI version")
    if run([about, "--version"], capture=True) != f"cargo-about {ABOUT_VERSION}":
        raise RuntimeError("Unexpected cargo-about version")
    return tauri, about


def license_inventory(about, target, stage):
    legal = stage / "legal"
    legal.mkdir(parents=True, exist_ok=True)
    runtime_legal = legal / "ocean-os"
    runtime_legal.mkdir(exist_ok=True)
    for name in LEGAL_FILES:
        shutil.copyfile(RUNTIME / name, runtime_legal / name)
    # Surface manifests already declare MIT. Carry the project's full MIT text.
    shutil.copyfile(RUNTIME / "LICENSE-MIT", legal / "SURFACE-LICENSE-MIT.txt")
    shutil.copyfile(RUNTIME / "LICENSE-APACHE", legal / "GOOGLE-CODE-APACHE.txt")
    shutil.copyfile(SURFACE / "public/fonts/OFL.txt", legal / "POPPINS-OFL.txt")
    shutil.copyfile(SURFACE / "packaging/ASSET-NOTICES.txt", legal / "ASSET-NOTICES.txt")
    graphs = (("daemon", RUNTIME / "crates/ocean-daemon", target),
              ("desktop", NATIVE, target),
              ("web-ui", SURFACE / "crates/ocean-surface-ui", "wasm32-unknown-unknown"))
    config_source = (RUNTIME / "packaging/about.toml").read_text(encoding="utf-8")
    notices = {}
    for name, package, graph_target in graphs:
        run(["cargo", "fetch", "--locked", "--manifest-path", package / "Cargo.toml"], cwd=package)
        config = stage / f"about-{name}.toml"
        config.write_text(re.sub(r'^targets = .*$', f'targets = ["{graph_target}"]', config_source, flags=re.M))
        text_output = legal / f"{name}-DEPENDENCIES.txt"
        json_output = stage / f"{name}-licenses.json"
        base = [about, "generate", "--frozen", "--fail", "--manifest-path", package / "Cargo.toml", "--config", config]
        run(base + ["--output-file", text_output, RUNTIME / "packaging/about.hbs"], cwd=package)
        run(base + ["--format", "json", "--output-file", json_output], cwd=package)
        report = json.loads(json_output.read_text(encoding="utf-8"))
        if not report["crates"] or not report["licenses"] or text_output.stat().st_size == 0:
            raise RuntimeError("Empty dependency license inventory")
        for crate in report["crates"]:
            package_info = crate["package"]
            root = Path(package_info["manifest_path"]).resolve().parent
            for directory, dirs, files in os.walk(root, followlinks=False):
                dirs[:] = sorted(d for d in dirs if d not in (".git", "target"))
                for filename in sorted(files):
                    if filename.upper() not in ("NOTICE", "NOTICE.MD", "NOTICE.TXT"):
                        continue
                    path = Path(directory) / filename
                    if not path.resolve().is_relative_to(root):
                        raise RuntimeError("Dependency notice escapes its package")
                    key = (package_info["name"], package_info["version"], path.relative_to(root).as_posix())
                    text = path.read_text(encoding="utf-8").rstrip()
                    if notices.setdefault(key, text) != text:
                        raise RuntimeError("Dependency notice changed during generation")
    (legal / "UPSTREAM-NOTICES.txt").write_text("\n\n".join(
        f"{name} {version} — {path}\n{text}" for (name, version, path), text in sorted(notices.items())), encoding="utf-8")
    return legal


def binary_architecture(path, target, gui=False):
    with Path(path).open("rb") as file:
        header = file.read(64)
        if "windows" in target:
            if len(header) != 64 or header[:2] != b"MZ":
                raise RuntimeError("Expected a Windows executable")
            pe = struct.unpack_from("<I", header, 60)[0]
            file.seek(pe)
            header = file.read(94)
            if len(header) != 94 or header[:4] != b"PE\0\0" or struct.unpack_from("<H", header, 4)[0] != 0x8664:
                raise RuntimeError("Expected an x64 PE executable")
            if gui and struct.unpack_from("<H", header, 92)[0] != 2:
                raise RuntimeError("Desktop executable would open a console")
        else:
            cpu = 0x0100000c if target.startswith("aarch64") else 0x01000007
            if len(header) < 8 or header[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", header, 4)[0] != cpu:
                raise RuntimeError("Unexpected Mach-O architecture")


def verify_web_bundle(directory):
    wasm = list(directory.glob("*.wasm"))
    if not wasm or not (directory / "index.html").is_file():
        raise RuntimeError("Missing web bundle")
    for path in wasm:
        with path.open("rb") as file:
            if file.read(8) != b"\0asm\x01\0\0\0" or path.stat().st_size <= 8:
                raise RuntimeError("Invalid or truncated WASM bundle")


@contextmanager
def mount_image(image, temporary):
    mount = temporary / "mounted"
    mount.mkdir()
    run(["hdiutil", "attach", image, "-readonly", "-nobrowse", "-mountpoint", mount])
    try:
        yield mount / "Ocean.app"
    finally:
        run(["hdiutil", "detach", mount])


def verify_payload(app_root, daemon, legal, target):
    suffix = ".exe" if "windows" in target else ""
    binaries = app_root if suffix else app_root / "Contents/MacOS"
    resources = app_root if suffix else app_root / "Contents/Resources"
    bundled_daemon = binaries / ("ocean-daemon" + suffix)
    binary_architecture(binaries / ("ocean-tauri" + suffix), target, gui=True)
    binary_architecture(bundled_daemon, target)
    if digest(bundled_daemon) != digest(daemon):
        raise RuntimeError("Bundled daemon does not match this source build")
    for file in legal.rglob("*"):
        if file.is_file() and digest(resources / "legal" / file.relative_to(legal)) != digest(file):
            raise RuntimeError("Bundled legal payload differs from its inventory")
    return bundled_daemon


def build(target):
    expected_os, expected_arch = TARGETS[target]
    if (platform.system(), platform.machine().lower()) != (expected_os, expected_arch):
        raise RuntimeError("Build this candidate on its native architecture")
    revision = clean_revision()
    stage = SURFACE / "target/desktop" / revision / target
    output = stage / "artifacts"
    if output.exists():
        raise RuntimeError("This immutable candidate output already exists")
    stage.mkdir(parents=True, exist_ok=True)
    tauri, about = tools(target)
    legal = license_inventory(about, target, stage)
    run(["cargo", "build", "--locked", "--release", "--target", target, "-p", "ocean-daemon"], cwd=RUNTIME)
    suffix = ".exe" if "windows" in target else ""
    daemon = RUNTIME / "target" / target / "release" / ("ocean-daemon" + suffix)
    binary_architecture(daemon, target)
    run(["trunk", "build", "--locked", "--release"], cwd=SURFACE)
    verify_web_bundle(SURFACE / "dist")
    sidecars = stage / "sidecars"
    sidecars.mkdir(exist_ok=True)
    shutil.copy2(daemon, sidecars / (f"ocean-daemon-{target}" + suffix))
    config = stage / "bundle.json"
    config.write_text(json.dumps({"bundle": {
        "externalBin": [str(sidecars / "ocean-daemon")],
        "resources": {str(legal) + os.sep: "legal/"},
        "windows": {"nsis": {"installMode": "currentUser"}},
    }}))
    run([tauri, "build", "--ci", "--no-sign", "--target", target, "--bundles",
         "nsis" if suffix else "app,dmg", "--config", config, "--", "--locked"], cwd=NATIVE)
    bundles = NATIVE / "target" / target / "release/bundle"
    with tempfile.TemporaryDirectory(prefix="verify-", dir=stage) as temporary:
        temporary = Path(temporary)
        if suffix:
            installers = list((bundles / "nsis").glob("*.exe"))
            if len(installers) != 1:
                raise RuntimeError("Expected one Windows installer")
            extractor = shutil.which("7z") or str(Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "7-Zip/7z.exe")
            run([extractor, "x", installers[0], f"-o{temporary / 'unpacked'}", "-y"])
            bundled_daemon = verify_payload(temporary / "unpacked", daemon, legal, target)
        else:
            installers = list((bundles / "dmg").glob("*.dmg"))
            if len(installers) != 1:
                raise RuntimeError("Expected one Mac disk image")
            with mount_image(installers[0], temporary) as app:
                bundled_daemon = verify_payload(app, daemon, legal, target)
                run([os.sys.executable, ROOT / ".github/test-daemon-startup.py", bundled_daemon, revision])
        # The fixture strips provider credentials, uses disposable state, checks
        # source revision/member identity and stops only the child it creates.
        if suffix:
            run([os.sys.executable, ROOT / ".github/test-daemon-startup.py", bundled_daemon, revision])
        if clean_revision() != revision:
            raise RuntimeError("Source changed during packaging")
        prepared = temporary / "artifacts"
        prepared.mkdir()
        for installer in installers:
            shutil.copy2(installer, prepared / installer.name)
        receipt = {"revision": revision, "target": target, "signing": "unsigned",
                   "daemon_sha256": digest(daemon),
                   "files": {p.name: digest(p) for p in sorted(prepared.iterdir())}}
        (prepared / "candidate.json").write_text(json.dumps(receipt, indent=2) + "\n")
        prepared.rename(output)
    print(f"Candidate: {output}", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    build(parser.parse_args().target)
