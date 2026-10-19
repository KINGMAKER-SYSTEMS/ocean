#!/usr/bin/env python3
"""Boot a candidate daemon in an isolated profile without provider credentials.

This checks startup and identity, not provider inference or installation.
Windows ACL security is exercised by the separate native Rust fixtures.
"""
import argparse, json, os, pathlib, socket, subprocess, tempfile, time, urllib.request, uuid

parser = argparse.ArgumentParser()
parser.add_argument('binary', type=pathlib.Path)
parser.add_argument('revision')
args = parser.parse_args()
binary = args.binary.resolve(strict=True)
root = pathlib.Path(tempfile.mkdtemp(prefix='ocean-release-runtime-'))
config = root / 'config'
config.mkdir(mode=0o700)
member = 'release-smoke-' + uuid.uuid4().hex
(config / 'member.toml').write_text(f'member_id = "{member}"\ndisplay_name = "Release smoke"\n')
(config / 'member.toml').chmod(0o600)
with socket.socket() as reservation:
    reservation.bind(('127.0.0.1', 0))
    port = reservation.getsockname()[1]
# Preserve OS identity, never provider credentials. Both auth-file discovery
# paths and all runtime state point into this disposable profile.
env = {key: os.environ[key] for key in ('PATH', 'HOME', 'USERPROFILE', 'SYSTEMROOT', 'TEMP', 'TMP', 'TMPDIR', 'LANG', 'LC_ALL') if key in os.environ}
env.update(OCEAN_CONFIG_DIR=str(config), XDG_CONFIG_HOME=str(root / 'xdg'),
           OCEAN_AUTH_FILE=str(config / 'auth.json'), OCEAN_CODEX_AUTH_FILE=str(config / 'codex-auth.json'),
           OCEAN_BIND=f'127.0.0.1:{port}', OCEAN_UNSUPERVISED='1', OCEAN_MODEL='gpt-5.5')
with (root / 'daemon.log').open('wb') as log:
    child = subprocess.Popen([str(binary)], cwd=root, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 45
        health = None
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError(f'candidate exited {child.returncode}; log: {root / "daemon.log"}')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/health', timeout=1) as response:
                    health = json.load(response)
                break
            except (OSError, ValueError):
                time.sleep(.2)
        if health is None:
            raise RuntimeError(f'candidate did not become healthy; log: {root / "daemon.log"}')
        with urllib.request.urlopen(f'http://127.0.0.1:{port}/v1/identity', timeout=3) as response:
            identity = json.load(response)
        assert identity['member_id'] == member, 'listener does not belong to the disposable candidate profile'
        assert health['ok'] is True and health['service'] == 'ocean-daemon'
        assert health['rev'] and (health['rev'] == args.revision or args.revision.startswith(health['rev'])), health['rev']
        for name in ('operator.key', 'observatory-secret', 'observatory-token', 'rooms.db'):
            path = config / name
            assert path.is_file(), f'missing startup state: {name}'
            if os.name == 'posix':
                assert path.stat().st_mode & 0o777 == 0o600, f'unsafe startup permissions: {name}'
        checks = ['fresh-profile startup', 'health', 'isolated member identity', 'startup state files exist']
        if os.name == 'posix':
            checks.append('private startup file modes')
        result = {'result': 'PASS', 'revision': health['rev'], 'checks': checks, 'profile': str(root), 'provider_reply': 'not tested'}
        (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
