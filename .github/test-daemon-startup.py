#!/usr/bin/env python3
"""Boot a candidate daemon in an isolated profile without provider credentials.

This checks startup and identity, not provider inference or installation.
Windows ACL security is exercised by the separate native Rust fixtures.
"""
import argparse, json, os, pathlib, socket, subprocess, tempfile, time, urllib.error, urllib.request, uuid

parser = argparse.ArgumentParser()
parser.add_argument('binary', type=pathlib.Path)
parser.add_argument('revision')
parser.add_argument('--mcp', type=pathlib.Path, help='Also verify this bundled MCP bridge against the isolated daemon')
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
           CLAUDE_CONFIG_DIR=str(root / 'claude'),
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
        if args.mcp is not None:
            bridge = args.mcp.resolve(strict=True)
            requests = [
                {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}},
                {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
                {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
                {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {'name': 'ocean_health', 'arguments': {}}},
                {'jsonrpc': '2.0', 'id': 4, 'method': 'tools/call', 'params': {'name': 'ocean_rooms', 'arguments': {}}},
            ]
            completed = subprocess.run(
                [str(bridge), '--daemon', f'http://127.0.0.1:{port}', 'serve'],
                input=''.join(json.dumps(request) + '\n' for request in requests),
                cwd=root, env=env, text=True, encoding="utf-8", capture_output=True, timeout=30)
            (root / "mcp.stdout.log").write_text(completed.stdout, encoding="utf-8")
            (root / "mcp.stderr.log").write_text(completed.stderr, encoding="utf-8")
            completed.check_returncode()
            replies = [json.loads(line) for line in completed.stdout.splitlines()]
            assert [reply.get('id') for reply in replies] == [1, 2, 3, 4], 'invalid MCP response sequence'
            assert replies[0]['result']['serverInfo']['name'] == 'ocean'
            names = {tool['name'] for tool in replies[1]['result']['tools']}
            assert {'ocean_health', 'ocean_rooms', 'ocean_room_post', 'ocean_sessions'} <= names
            assert all(reply['result']['isError'] is False for reply in replies[2:])
            projected_health = json.loads(replies[2]['result']['content'][0]['text'])
            assert projected_health['rev'] == health['rev'], 'MCP bridge reached a different daemon'
            assert projected_health['service'] == 'ocean-daemon'
            projected_rooms = json.loads(replies[3]['result']['content'][0]['text'])
            with urllib.request.urlopen(f'http://127.0.0.1:{port}/v1/rooms/persistent?limit=20', timeout=3) as response:
                expected_rooms = json.load(response)
            assert [room['room']['id'] for room in projected_rooms['rooms']] == [room['id'] for room in expected_rooms['rooms']]
            checks += ['bundled MCP stdio handshake', 'MCP daemon revision', 'MCP room projection']
        # Exercise the packaged backend's initial team configuration without
        # joining a service, sharing credentials or touching the operator profile.
        def team_request(method='GET', origin=None, authorized=True):
            headers = {'Content-Type': 'application/json'}
            if authorized:
                headers['X-Ocean-Operator'] = (config / 'operator.key').read_text().strip()
            data = None if origin is None else json.dumps({'origin': origin}).encode()
            request = urllib.request.Request(f'http://127.0.0.1:{port}/v1/rooms/connection',
                                             data=data, headers=headers, method=method)
            try:
                with urllib.request.urlopen(request, timeout=3) as response:
                    return response.status, json.load(response)
            except urllib.error.HTTPError as error:
                return error.code, json.load(error)

        assert team_request(authorized=False)[0] == 503, 'team settings require operator authority'
        status, initial = team_request()
        assert status == 200 and initial['connection'] == {
            'origin': None, 'configured': False, 'restart_required': False, 'can_initialize': True}
        status, saved = team_request('PUT', 'https://team.example')
        assert status == 200 and saved['connection'] == {
            'origin': 'https://team.example/', 'configured': False,
            'restart_required': True, 'can_initialize': False}
        child.terminate()
        child.wait(timeout=15)
        child = subprocess.Popen([str(binary)], cwd=root, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        deadline = time.monotonic() + 45
        restarted = None
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError(f'candidate restart exited {child.returncode}; log: {root / "daemon.log"}')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/v1/identity', timeout=1) as response:
                    assert json.load(response)['member_id'] == member
                status, restarted = team_request()
                if status == 200:
                    break
            except (OSError, ValueError):
                time.sleep(.2)
        assert restarted and restarted.get('connection') == {
            'origin': 'https://team.example/', 'configured': True,
            'restart_required': False, 'can_initialize': False}, 'saved team origin did not activate'
        assert team_request('PUT', 'https://different.example')[0] == 409, 'existing origin was replaceable'
        assert team_request()[1]['connection']['origin'] == 'https://team.example/'
        checks += ['operator-gated team setup', 'saved team origin survives restart', 'existing team origin cannot be replaced']
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
