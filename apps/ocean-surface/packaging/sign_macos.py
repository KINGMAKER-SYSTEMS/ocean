#!/usr/bin/env python3
"""Prepare a Developer ID-signed copy of a verified Mac candidate for notarization.

Never installs, publishes, exports keys or changes Keychain access. The result is
explicitly not notarized; the original candidate is left untouched.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile

import build_desktop as desktop


def read_candidate(directory, revision):
    directory = Path(directory).resolve(strict=True)
    manifest = directory / 'candidate.json'
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 65536:
        raise RuntimeError('Expected a bounded regular candidate receipt')
    receipt = json.loads(manifest.read_text(encoding='utf-8'))
    if (not isinstance(receipt, dict) or not re.fullmatch(r'[0-9a-f]{40}', revision)
            or receipt.get('revision') != revision
            or receipt.get('signing') != 'unsigned'
            or receipt.get('target') not in ('aarch64-apple-darwin', 'x86_64-apple-darwin')):
        raise RuntimeError('Candidate revision, platform or unsigned provenance does not match')
    files = receipt.get('files')
    if not isinstance(files, dict) or len(files) != 1:
        raise RuntimeError('Expected one candidate disk image')
    name, checksum = next(iter(files.items()))
    if (not isinstance(name, str) or Path(name).name != name or '/' in name or '\\' in name
            or not name.endswith('.dmg') or not isinstance(checksum, str)
            or not re.fullmatch(r'[0-9a-f]{64}', checksum)):
        raise RuntimeError('Invalid candidate artifact name or checksum')
    image = directory / name
    if image.is_symlink() or not image.is_file() or desktop.digest(image) != checksum:
        raise RuntimeError('Candidate disk image checksum does not match')
    for key in ('daemon_sha256', 'mcp_sha256'):
        if not isinstance(receipt.get(key), str) or not re.fullmatch(r'[0-9a-f]{64}', receipt[key]):
            raise RuntimeError('Missing sidecar checksum')
    return receipt, image


def signature_metadata(report, team):
    lines = report.splitlines()
    flags = [re.search(r'\bflags=0x([0-9a-fA-F]+)\b', line) for line in lines if line.startswith('CodeDirectory ')]
    runtime = any(match and int(match.group(1), 16) & 0x10000 for match in flags)
    if (f'TeamIdentifier={team}' not in lines
            or not any(line.startswith('Authority=Developer ID Application:') for line in lines)
            or not runtime):
        raise RuntimeError('Signature lacks the expected Developer ID team or hardened runtime')
    timestamps = [line.removeprefix('Timestamp=') for line in lines if line.startswith('Timestamp=')]
    if len(timestamps) != 1 or not timestamps[0].strip() or timestamps[0].lower() == 'none':
        raise RuntimeError('Signature lacks a secure timestamp')
    return {'team': team, 'hardened_runtime': True, 'timestamp': timestamps[0]}


def command(args, timeout=60):
    result = subprocess.run(list(map(str, args)), check=False, capture_output=True,
                            text=True, encoding='utf-8', timeout=timeout)
    if result.returncode:
        if 'errSecInternalComponent' in result.stderr or 'User interaction is not allowed' in result.stderr:
            raise RuntimeError('macOS could not use this signing identity. Check certificate trust and Keychain access locally; no permissions were changed.')
        raise RuntimeError(f'{Path(str(args[0])).name} failed with exit status {result.returncode}')
    return result.stdout + result.stderr


def prepare(directory, revision, identity, team, output):
    if platform.system() != 'Darwin':
        raise RuntimeError('Mac candidate signing requires macOS')
    if not re.fullmatch(r'[0-9A-Fa-f]{40}', identity) or not re.fullmatch(r'[A-Z0-9]{10}', team):
        raise RuntimeError('Provide a certificate SHA-1 and its Apple team id')
    receipt, image = read_candidate(directory, revision)
    output = Path(output).absolute()
    if output.exists() or output.is_symlink():
        raise RuntimeError('Signing output already exists; choose a new directory')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='ocean-sign-', dir=output.parent) as temporary:
        temporary = Path(temporary)
        # Snapshot and verify before mounting: later edits to the input cannot
        # replace the bytes being signed. No source artifact is ever modified.
        snapshot = temporary / 'candidate.dmg'
        with image.open('rb') as source, snapshot.open('xb') as destination:
            shutil.copyfileobj(source, destination)
        if desktop.digest(snapshot) != receipt['files'][image.name]:
            raise RuntimeError('Candidate changed while copying')
        prepared = temporary / 'prepared'
        prepared.mkdir()
        with desktop.mount_image(snapshot, temporary) as mounted:
            if mounted.is_symlink() or not mounted.is_dir():
                raise RuntimeError('Candidate does not contain a regular Ocean app')
            command(['/usr/bin/ditto', mounted, prepared / 'Ocean.app'])
        app = prepared / 'Ocean.app'
        binaries = [app / 'Contents/MacOS' / name for name in ('ocean-daemon', 'ocean-mcp', 'ocean-tauri')]
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        if info.get('CFBundleIdentifier') != 'dev.ocean.surface' or info.get('CFBundleExecutable') != 'ocean-tauri':
            raise RuntimeError('Unexpected application identity')
        for binary in binaries:
            if binary.is_symlink() or not binary.is_file():
                raise RuntimeError('Expected regular bundled executables')
            desktop.binary_architecture(binary, receipt['target'])
            desktop.verify_macos_libraries(command(['/usr/bin/otool', '-L', binary]))
        for binary, key in zip(binaries[:2], ('daemon_sha256', 'mcp_sha256')):
            if desktop.digest(binary) != receipt[key]:
                raise RuntimeError('Bundled sidecar checksum does not match')
        # This candidate contract has three executables and no extra native
        # frameworks/plugins. Fail instead of silently leaving nested code unsigned.
        for path in app.rglob('*'):
            if path.is_symlink():
                raise RuntimeError('Unexpected link inside the candidate app')
            if path.is_file() and path not in binaries:
                with path.open('rb') as file:
                    if file.read(4).hex() in ('cffaedfe', 'cefaedfe', 'feedfacf', 'feedface', 'cafebabe', 'bebafeca', 'cafebabf', 'bfbafeca'):
                        raise RuntimeError('Unexpected nested native code')
        for target in [*binaries[:2], app]:
            command(['/usr/bin/codesign', '--force', '--sign', identity, '--options', 'runtime', '--timestamp', target])
        signatures = {}
        for target in [*binaries, app]:
            command(['/usr/bin/codesign', '--verify', '--strict', '--verbose=2', target])
            signatures[target.name] = signature_metadata(command(['/usr/bin/codesign', '--display', '--verbose=4', target]), team)
        command(['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=2', app])
        archive = prepared / 'Ocean.zip'
        command(['/usr/bin/ditto', '-c', '-k', '--keepParent', app, archive])
        result = {'source_candidate': receipt, 'signing_identity_sha1': identity.upper(),
                  'signatures': signatures, 'notarization': 'not_submitted',
                  'archive_sha256': desktop.digest(archive),
                  'signed_binaries': {p.name: desktop.digest(p) for p in binaries}}
        (prepared / 'signing.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
        if output.exists() or output.is_symlink():
            raise RuntimeError('Signing output appeared during preparation')
        prepared.rename(output)
    return output


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--identity', required=True, help='Developer ID certificate SHA-1, not a password')
    parser.add_argument('--team-id', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        result = prepare(args.candidate, args.revision, args.identity, args.team_id, args.output)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f'{error}\n')
    print(f'Signed copy prepared at {result}; notarization is still required.')
