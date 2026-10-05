import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, readlinkSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const repo = resolve(import.meta.dirname, '..');
const script = join(repo, 'deploy', 'ocean-surface-auto-deploy.sh');
const proxyPlist = readFileSync(join(repo, 'deploy', 'dev.risingtides.ocean-surface-proxy.plist'), 'utf8');
const deployPlist = readFileSync(join(repo, 'deploy', 'dev.risingtides.ocean-surface-auto-deploy.plist'), 'utf8');
const root = mkdtempSync(join(tmpdir(), 'ocean-surface-promote-'));
const railSrc = readFileSync(script, 'utf8');
const releaseSrc = readFileSync(join(repo, 'deploy/surface-release.sh'), 'utf8');
const proxySrc = readFileSync(join(repo, 'deploy/ocean-surface-proxy.sh'), 'utf8');
const installerSrc = readFileSync(join(repo, 'ops/install-surface-proxy.sh'), 'utf8');

function fixtureFile(path, content) {
  mkdirSync(resolve(path, '..'), { recursive: true });
  writeFileSync(path, content);
}

function commitFixture(fixture, message) {
  execFileSync('git', ['add', '.'], { cwd: fixture.gitRoot, stdio: 'pipe' });
  execFileSync('git', ['commit', '-qm', message], { cwd: fixture.gitRoot, stdio: 'pipe' });
  return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: fixture.gitRoot, encoding: 'utf8' }).trim();
}

function shellQuote(value) {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

// Real Git repositories and detached worktrees exercise path resolution and
// promotion and actual launchers. Rust/Trunk tools check cwd and target cache;
// Node runs a real component-local gate. Fixture launchctl and curl model
// supervision/health without installing a service or contacting a listener.
function deploymentFixture(name, prefix) {
  const gitRoot = join(root, name);
  const surface = join(gitRoot, prefix);
  const ownerHome = join(root, `${name}-ownerHome`);
  const commandLog = join(root, `${name}-commands`);
  const fixture = { gitRoot, surface, prefix, commandLog, state: join(root, `${name}-state`) };
  fixtureFile(join(surface, 'Cargo.toml'), '[workspace]\n');
  fixtureFile(join(surface, 'Trunk.toml'), '[build]\ndist = "dist"\n');
  fixtureFile(join(surface, 'crates/ocean-surface-ui/Cargo.toml'), '[package]\nname = "ocean-surface-ui"\n');
  fixtureFile(join(surface, 'crates/ocean-tauri/src/lib.rs'), '// initial native shell\n');
  fixtureFile(join(surface, 'extension/sidepanel.html'), '<script src="dist/ocean-surface-ui.js"></script>');
  fixtureFile(join(surface, '.gitignore'), '/dist/\n/target/\n/extension/dist/\n');
  fixtureFile(join(surface, 'build-content'), '<p>first component build</p>');
  fixtureFile(join(surface, 'scripts/surface-auto-deploy.test.mjs'), `
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
assert.ok(existsSync('Trunk.toml') && existsSync('crates/ocean-surface-ui/Cargo.toml'));
`);
  fixture.script = join(surface, 'deploy/ocean-surface-auto-deploy.sh');
  fixtureFile(fixture.script, railSrc);
  fixtureFile(join(surface, 'deploy/surface-release.sh'), releaseSrc);
  fixtureFile(join(surface, 'deploy/ocean-surface-proxy.sh'), proxySrc);
  chmodSync(join(surface, 'deploy/ocean-surface-proxy.sh'), 0o755);
  chmodSync(fixture.script, 0o755);
  execFileSync('git', ['init', '-q', '-b', 'main'], { cwd: gitRoot, stdio: 'pipe' });
  execFileSync('git', ['config', 'user.email', 'fixture@example.invalid'], { cwd: gitRoot });
  execFileSync('git', ['config', 'user.name', 'Deployment fixture'], { cwd: gitRoot });
  fixture.initialRevision = commitFixture(fixture, 'Initial Surface fixture');

  const toolPreamble = `#!/usr/bin/env bash
set -euo pipefail
prefix="$(git rev-parse --show-prefix)"
[[ "$prefix" == "$FIXTURE_SURFACE_PREFIX" && -f Trunk.toml ]] || { echo 'wrong build directory' >&2; exit 91; }
[[ "$CARGO_TARGET_DIR" == "$FIXTURE_TARGET_DIR" ]] || { echo 'wrong component target cache' >&2; exit 92; }
printf '%s|%s|%s\\n' "$(basename "$0")" "$prefix" "$*" >> "$FIXTURE_COMMAND_LOG"
`;
  const tools = {
    cargo: `${toolPreamble}if [[ "\${FIXTURE_FAIL_BUILD:-0}" == "1" ]]; then exit 7; fi
if [[ "$*" == 'build -p ocean-surface-proxy --release' ]]; then
  mkdir -p "$CARGO_TARGET_DIR/release"
  printf '#!/usr/bin/env bash\\nprintf "fixture proxy %s dist=%%s\\n" "$OCEAN_SURFACE_DIST"\\n' "$(git rev-parse HEAD)" > "$CARGO_TARGET_DIR/release/ocean-surface-proxy"
  cat >> "$CARGO_TARGET_DIR/release/ocean-surface-proxy" <<'PROXY'
python3 -c 'import json,os; print(json.dumps({"ok": True, "service": "ocean-surface-proxy", "release_revision": os.environ.get("OCEAN_SURFACE_RELEASE_REVISION"), "proxy_sha256": os.environ.get("OCEAN_SURFACE_RELEASE_PROXY_SHA256")}))' > "$FIXTURE_HEALTH_FILE"
PROXY
  chmod 755 "$CARGO_TARGET_DIR/release/ocean-surface-proxy"
  case "\${FIXTURE_SOURCE_CHANGE:-}" in
    dirty) printf 'changed during build' >> build-content ;;
    untracked) printf 'new source during build' > new-source.rs ;;
    head) git commit --allow-empty -qm 'HEAD changed during build' ;;
  esac
fi
`,
    trunk: `${toolPreamble}
[[ "$*" == 'build --release' ]]
mkdir -p dist
cp build-content dist/index.html
printf '\\x00asm\\x01' > dist/ocean-surface-ui_bg.wasm
printf '// component release glue' > dist/ocean-surface-ui.js
printf ':root { color: blue; }' > dist/tokens-abc123.css
`,
    node: `${toolPreamble}exec ${shellQuote(process.execPath)} "$@"\n`,
    curl: `#!/usr/bin/env bash\n[[ -f "$FIXTURE_HEALTH_FILE" ]] || exit 22\ncat "$FIXTURE_HEALTH_FILE"\n`,
    sleep: '#!/usr/bin/env bash\nexit 0\n',

  };
  for (const [name, source] of Object.entries(tools)) {
    const executable = join(ownerHome, '.cargo/bin', name);
    fixtureFile(executable, source);
    chmodSync(executable, 0o755);
  }
  fixture.env = {
    ...process.env,
    OCEAN_SURFACE_OWNER_HOME: ownerHome,
    CARGO_TARGET_DIR: '',
    OCEAN_SURFACE_STATE_DIR: fixture.state,
    OCEAN_SURFACE_WORKTREE_ROOT: join(root, `${name}-worktree`),
    OCEAN_SURFACE_NO_RESTART: '1',
    FIXTURE_SURFACE_PREFIX: prefix,
    FIXTURE_TARGET_DIR: join(realpathSync(surface), 'target'),
    FIXTURE_COMMAND_LOG: commandLog,
    FIXTURE_HEALTH_FILE: join(root, `${name}-health.json`),
    FIXTURE_FAILURE_FILE: join(root, `${name}-restart-failed`),
  };
  delete fixture.env.OCEAN_SURFACE_REPO;
  delete fixture.env.OCEAN_SURFACE_TARGET_REV;
  delete fixture.env.FIXTURE_FAIL_BUILD;
  fixtureFile(join(surface, 'target/release/ocean-surface-proxy'), '#!/usr/bin/env bash\nprintf "legacy fixture proxy dist=%s\\n" "$OCEAN_SURFACE_DIST"\n');
  chmodSync(join(surface, 'target/release/ocean-surface-proxy'), 0o755);
  return fixture;
}

function installerFixture(name, prefix) {
  const fixture = deploymentFixture(name, prefix);
  const ownerHome = fixture.env.OCEAN_SURFACE_OWNER_HOME;
  fixture.state = join(ownerHome, '.config/ocean-surface');
  fixture.env.OCEAN_SURFACE_STATE_DIR = fixture.state;
  fixture.installer = join(fixture.surface, 'ops/install-surface-proxy.sh');
  fixture.launchLog = join(root, `${name}-launchctl`);
  fixture.supervisionState = join(root, `${name}-supervision.json`);
  fixture.env.FIXTURE_SUPERVISION_STATE = fixture.supervisionState;
  fixture.env.FIXTURE_LAUNCH_LOG = fixture.launchLog;
  writeFileSync(fixture.supervisionState, JSON.stringify({}));
  fixtureFile(fixture.installer, installerSrc);
  chmodSync(fixture.installer, 0o755);
  fixtureFile(join(fixture.surface, 'deploy/dev.risingtides.ocean-surface-proxy.plist'), proxyPlist);
  fixtureFile(join(fixture.surface, 'deploy/dev.risingtides.ocean-surface-auto-deploy.plist'), deployPlist);
  fixtureFile(join(fixture.surface, 'deploy/ocean-surface-proxy.sh'), proxySrc);
  chmodSync(join(fixture.surface, 'deploy/ocean-surface-proxy.sh'), 0o755);
  fixtureFile(join(ownerHome, '.config/ocean-surface/proxy-auth.env'), 'OCEAN_SURFACE_AUTH=on\nOCEAN_SURFACE_USER=fixture-operator\nOCEAN_SURFACE_PASS=fixture-secret\n');
  chmodSync(join(ownerHome, '.config/ocean-surface/proxy-auth.env'), 0o600);
  const launchctl = join(ownerHome, '.cargo/bin/launchctl');
  const launchModel = join(ownerHome, 'launchctl-model.py');
  fixtureFile(launchModel, `import json, os, pathlib, plistlib, subprocess, sys
args = sys.argv[1:]
with open(os.environ['FIXTURE_LAUNCH_LOG'], 'a') as log:
    log.write(' '.join(args) + '\\n')
path = pathlib.Path(os.environ['FIXTURE_SUPERVISION_STATE'])
states = json.loads(path.read_text())
label = args[-1].rsplit('/', 1)[-1]
state = states.setdefault(label, {'loaded': False, 'disabled': False})
command = args[0]
if command == 'print-disabled':
    print('disabled services = {')
    for key, value in states.items():
        if key.startswith('dev.risingtides.ocean-surface-'):
            disabled = value['disabled']
            print('    "' + key + '" => ' + (str(disabled).lower() if isinstance(disabled, bool) else str(disabled)))
    print('}')
    sys.exit(0)
if command == 'print':
    sys.exit(0 if args[-1].count('/') == 1 or state['loaded'] else 113)
if command in ['enable', 'disable']:
    sys.exit(99)
if command == 'bootout':
    state['loaded'] = False
elif command == 'bootstrap':
    with open(args[-1], 'rb') as source:
        config = plistlib.load(source)
    label = config['Label']
    state = states.setdefault(label, {'loaded': False, 'disabled': False})
    if state['disabled']:
        sys.exit(78)
    if os.environ.get('FIXTURE_FAIL_RECOVERY_BOOTSTRAP') == '1' and pathlib.Path(os.environ['FIXTURE_FAILURE_FILE']).exists():
        sys.exit(17)
    state['loaded'] = True
elif command == 'kickstart':
    if not state['loaded']:
        sys.exit(113)
    failed = pathlib.Path(os.environ['FIXTURE_FAILURE_FILE'])
    if os.environ.get('FIXTURE_FAIL_RESTART_ONCE') == '1' and not failed.exists():
        failed.touch()
        if os.environ.get('FIXTURE_CORRUPT_CAPTURE') == '1':
            (pathlib.Path(os.environ['OCEAN_SURFACE_STATE_DIR']) / 'legacy-recovery/proxy-sha256').write_text('invalid captured checksum')
        if os.environ.get('FIXTURE_CORRUPT_PREVIOUS_PROXY'):
            pathlib.Path(os.environ['FIXTURE_CORRUPT_PREVIOUS_PROXY']).write_text('invalid prior checksum')
        sys.exit(14)
    if os.environ.get('FIXTURE_SKIP_PROXY_EXEC') == '1':
        sys.exit(0)
    launcher = pathlib.Path(os.environ['OCEAN_SURFACE_OWNER_HOME']) / '.config/ocean-surface/bin/ocean-surface-proxy.sh'
    sys.exit(subprocess.run([str(launcher)]).returncode)
path.write_text(json.dumps(states))
`);
  fixtureFile(launchctl, `#!/usr/bin/env bash\nexec python3 ${shellQuote(launchModel)} "$@"\n`);
  chmodSync(launchctl, 0o755);
  // Exercise plist validation without touching macOS launchd, including on
  // hosts where plutil is unavailable. plistlib also parses the outputs below.
  const plutil = join(ownerHome, '.cargo/bin/plutil');
  fixtureFile(plutil, `#!/usr/bin/env bash
[[ "$1" == '-lint' ]] || exit 2
exec python3 -c 'import plistlib,sys; plistlib.load(open(sys.argv[1], "rb"))' "$2"
`);
  chmodSync(plutil, 0o755);
  fixture.initialRevision = commitFixture(fixture, 'Installer fixture');
  return fixture;
}

function installFixture(fixture, args = [], extraEnv = {}) {
  const result = spawnSync(fixture.installer, args, { env: { ...fixture.env, ...extraEnv }, cwd: root, encoding: 'utf8', timeout: 30_000 });
  assert.equal(result.error, undefined, result.error?.message);
  return result;
}

function readPlist(path) {
  return JSON.parse(execFileSync('python3', ['-c', 'import json,plistlib,sys; print(json.dumps(plistlib.load(open(sys.argv[1], "rb"))))', path], { encoding: 'utf8' }));
}

function deployFixture(fixture, revision, source, extraEnv = {}) {
  const env = { ...fixture.env, OCEAN_SURFACE_TARGET_REV: revision, ...extraEnv };
  if (source) env.OCEAN_SURFACE_REPO = source;
  const result = spawnSync(fixture.script, [], { env, cwd: root, encoding: 'utf8', timeout: 30_000 });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(existsSync(`${fixture.env.OCEAN_SURFACE_WORKTREE_ROOT}-${revision}`), false, 'detached build worktree must be removed');
  assert.equal(existsSync(join(fixture.state, 'auto-deploy.lock')), false, 'deployment must release its lock');
  return result;
}

function validBundle(path) {
  mkdirSync(path, { recursive: true });
  writeFileSync(join(path, 'index.html'), '<script type="module" src="/ocean.js"></script>');
  writeFileSync(join(path, 'ocean-surface-ui_bg.wasm'), Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01]));
  writeFileSync(join(path, 'ocean-surface-ui.js'), '// wasm-bindgen glue');
}

function clientAssets(fixture) {
  const assets = {};
  function visit(path) {
    if (!existsSync(join(fixture.surface, path))) return;
    for (const entry of readdirSync(join(fixture.surface, path), { withFileTypes: true })) {
      const file = join(path, entry.name);
      if (entry.isDirectory()) visit(file);
      else assets[file] = readFileSync(join(fixture.surface, file)).toString('hex');
    }
  }
  visit('dist');
  visit('extension/dist');
  return assets;
}

function launchFixture(fixture) {
  return spawnSync(join(fixture.env.OCEAN_SURFACE_OWNER_HOME, '.config/ocean-surface/bin/ocean-surface-proxy.sh'), [], {
    env: fixture.env, encoding: 'utf8', timeout: 5_000,
  });
}

const proxyLabel = 'dev.risingtides.ocean-surface-proxy';
const autoLabel = 'dev.risingtides.ocean-surface-auto-deploy';
function supervision(fixture, proxyLoaded, autoLoaded, disabledLabel = null, disabled = true) {
  writeFileSync(fixture.supervisionState, JSON.stringify({
    [proxyLabel]: { loaded: proxyLoaded, disabled: disabledLabel === proxyLabel ? disabled : false },
    [autoLabel]: { loaded: autoLoaded, disabled: disabledLabel === autoLabel ? disabled : false },
  }));
}
function retainedBackups(state) {
  return readdirSync(state).filter(name => name.startsWith('.client-backup.'))
    .map(name => join(state, name)).filter(path => existsSync(join(path, 'recovery-pending')));
}

function legacyFixture(fixture, originalComponent = fixture.surface) {
  const legacy = join(fixture.state, 'releases/legacy-bundle');
  validBundle(legacy);
  fixtureFile(join(legacy, '.deploy-sha'), `${fixture.initialRevision}\n`);
  symlinkSync('releases/legacy-bundle', join(fixture.state, 'current'));
  fixtureFile(join(fixture.state, 'deployed-rev'), `${fixture.initialRevision}\n`);
  const binary = join(originalComponent, 'target/release/ocean-surface-proxy');
  fixtureFile(binary, `#!/usr/bin/env bash
printf 'original legacy proxy\\n'
printf '{"ok":true,"service":"ocean-surface-proxy"}\\n' > "$FIXTURE_HEALTH_FILE"
`);
  chmodSync(binary, 0o755);
  fixtureFile(join(fixture.state, 'bin/ocean-surface-proxy.sh'), `#!/usr/bin/env bash\nexec ${shellQuote(binary)}\n`);
  chmodSync(join(fixture.state, 'bin/ocean-surface-proxy.sh'), 0o755);
  fixtureFile(join(fixture.state, 'bin/ocean-surface-auto-deploy.sh'), '#!/usr/bin/env bash\nexit 0\n');
  for (const suffix of ['proxy', 'auto-deploy']) {
    const path = join(fixture.env.OCEAN_SURFACE_OWNER_HOME, `Library/LaunchAgents/dev.risingtides.ocean-surface-${suffix}.plist`);
    fixtureFile(path, execFileSync('python3', ['-c', `import plistlib,sys
def render(v):
    if isinstance(v,str): return v.replace('__OCEAN_HOME__',sys.argv[2]).replace('__OCEAN_SURFACE_REPO__',sys.argv[3])
    if isinstance(v,list): return [render(x) for x in v]
    if isinstance(v,dict): return {k:render(x) for k,x in v.items()}
    return v
sys.stdout.buffer.write(plistlib.dumps(render(plistlib.load(open(sys.argv[1],'rb')))))`, join(fixture.surface, `deploy/dev.risingtides.ocean-surface-${suffix}.plist`), fixture.env.OCEAN_SURFACE_OWNER_HOME, originalComponent]));
  }
  validBundle(join(fixture.surface, 'dist'));
  fixtureFile(join(fixture.surface, 'dist/.deploy-sha'), 'old-client\n');
  fixtureFile(join(fixture.surface, 'extension/dist/ocean-surface-ui.js'), '// old extension');
  fixtureFile(join(fixture.surface, 'extension/dist/ocean-surface-ui_bg.wasm'), Buffer.from([0, 97, 115, 109, 2]));
  return { legacy, binary };
}

try {
  const promotionFixture = deploymentFixture('promotion source', '');
  const promotionEnv = { ...promotionFixture.env, OCEAN_SURFACE_REPO: promotionFixture.gitRoot };
  const state = join(root, 'state');
  const staged = join(root, 'staged');
  validBundle(staged);

  execFileSync(script, ['--promote', staged, 'abc123'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state },
    stdio: 'pipe',
  });

  assert.equal(readlinkSync(join(state, 'current')), 'releases/abc123-proxy/dist');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'abc123');
  assert.equal(readFileSync(join(state, 'releases', 'abc123-proxy', 'dist', 'ocean-surface-ui_bg.wasm')).subarray(0, 4).toString('hex'), '0061736d');

  const staged2 = join(root, 'staged2');
  validBundle(staged2);
  execFileSync(script, ['--promote', staged2, 'def456'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state },
    stdio: 'pipe',
  });
  assert.equal(readlinkSync(join(state, 'current')), 'releases/def456-proxy/dist', 'a second promotion must replace the current symlink');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'def456');
  assert.deepEqual(readdirSync(join(state, 'releases', 'abc123-proxy', 'dist')).sort(), ['.deploy-sha', 'index.html', 'ocean-surface-ui.js', 'ocean-surface-ui_bg.wasm']);

  const bad = join(root, 'bad');
  mkdirSync(bad, { recursive: true });
  writeFileSync(join(bad, 'index.html'), '<h1>broken</h1>');
  const failed = spawnSync(script, ['--promote', bad, 'broken'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state },
    encoding: 'utf8',
  });

  assert.notEqual(failed.status, 0, 'invalid bundle promotion must fail');
  assert.equal(readlinkSync(join(state, 'current')), 'releases/def456-proxy/dist', 'failed promotion must preserve current release');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'def456', 'failed promotion must preserve marker');

  const inPlace = join(promotionFixture.surface, 'dist');
  validBundle(inPlace);
  execFileSync(script, ['--promote', inPlace, 'in-place'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state }, stdio: 'pipe',
  });
  assert.equal(readFileSync(join(inPlace, 'index.html'), 'utf8'), '<script type="module" src="/ocean.js"></script>', 'in-place promotion keeps its input');
  assert.equal(readFileSync(join(inPlace, '.deploy-sha'), 'utf8').trim(), 'in-place');
  assert.equal(readlinkSync(join(state, 'current')), 'releases/in-place-proxy/dist');

  // A failed client sync must not advance the selected release or marker.
  const syncFailure = join(root, 'sync-failure');
  validBundle(syncFailure);
  const fixtureRsync = join(promotionFixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/rsync');
  fixtureFile(fixtureRsync, `#!/usr/bin/env bash
[[ "$*" != *${shellQuote(`${promotionFixture.surface}/dist/`)}* ]] || exit 13
exec /usr/bin/rsync "$@"
`);
  chmodSync(fixtureRsync, 0o755);
  const failedSync = spawnSync(script, ['--promote', syncFailure, 'sync-failed'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state }, encoding: 'utf8',
  });
  assert.notEqual(failedSync.status, 0, 'failed client sync stops promotion');
  assert.equal(readlinkSync(join(state, 'current')), 'releases/in-place-proxy/dist');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'in-place');
  rmSync(fixtureRsync);

  const priorAssets = clientAssets(promotionFixture);
  fixtureFile(fixtureRsync, `#!/usr/bin/env bash
if [[ "$*" == *${shellQuote(`${promotionFixture.surface}/dist/`)}* ]]; then
  printf 'partial failed sync' > ${shellQuote(join(promotionFixture.surface, 'dist/index.html'))}
  exit 13
fi
exec /usr/bin/rsync "$@"
`);
  chmodSync(fixtureRsync, 0o755);
  const partialSync = spawnSync(script, ['--promote', syncFailure, 'partial-failed'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state }, encoding: 'utf8',
  });
  assert.notEqual(partialSync.status, 0);
  assert.deepEqual(clientAssets(promotionFixture), priorAssets, 'partial client writes must restore the previous assets');
  rmSync(fixtureRsync);

  const fixtureLaunchctl = join(promotionFixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/launchctl');
  fixtureFile(fixtureLaunchctl, '#!/usr/bin/env bash\nexit 14\n');
  chmodSync(fixtureLaunchctl, 0o755);
  const failedRestart = spawnSync(script, ['--promote', syncFailure, 'restart-failed'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state, OCEAN_SURFACE_NO_RESTART: '0' }, encoding: 'utf8',
  });
  assert.notEqual(failedRestart.status, 0, 'failed proxy restart stops promotion');
  assert.equal(readlinkSync(join(state, 'current')), 'releases/in-place-proxy/dist', 'restart failure restores previous selected release');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'in-place', 'restart failure restores previous marker');
  assert.deepEqual(clientAssets(promotionFixture), priorAssets, 'restart failure restores already-copied client assets');
  assert.equal(readdirSync(state).some(name => name.startsWith('.current.') || name.startsWith('.deployed-rev.')), false);
  rmSync(fixtureLaunchctl);

  const noOpState = join(root, 'noop-state');
  const mainRevision = promotionFixture.initialRevision;
  execFileSync(script, ['--promote', staged, mainRevision], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: noOpState },
    stdio: 'pipe',
  });
  mkdirSync(join(noOpState, 'auto-deploy.lock'));
  writeFileSync(join(noOpState, 'auto-deploy.lock', 'pid'), '99999999\n');
  const noOp = spawnSync(script, [], {
    env: {
      ...promotionEnv,
      OCEAN_SURFACE_STATE_DIR: noOpState,
      OCEAN_SURFACE_NO_RESTART: '1',
      OCEAN_SURFACE_TARGET_REV: mainRevision,
    },
    encoding: 'utf8',
  });
  assert.equal(noOp.status, 0, noOp.stderr);
  assert.match(noOp.stdout, new RegExp(`CURRENT: ${mainRevision}`));
  assert.equal(existsSync(join(noOpState, 'auto-deploy.lock')), false, 'a stale deployment lock must be reclaimed');

  mkdirSync(join(noOpState, 'auto-deploy.lock'));
  writeFileSync(join(noOpState, 'auto-deploy.lock/pid'), `${process.pid}\n`);
  const busy = spawnSync(script, ['--promote', staged, 'lock-rejected'], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: noOpState }, encoding: 'utf8',
  });
  assert.equal(busy.status, 75, 'explicit promotion shares the scheduled deployment lock');
  const busyTick = spawnSync(script, [], {
    env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: noOpState, OCEAN_SURFACE_TARGET_REV: mainRevision }, encoding: 'utf8',
  });
  assert.equal(busyTick.status, 0, 'a scheduled tick leaves an active owner alone');
  assert.equal(readlinkSync(join(noOpState, 'current')), `releases/${mainRevision}-proxy/dist`);
  assert.equal(existsSync(join(noOpState, 'releases/lock-rejected-proxy')), false);
  assert.equal(readFileSync(join(noOpState, 'auto-deploy.lock/pid'), 'utf8').trim(), String(process.pid));
  rmSync(join(noOpState, 'auto-deploy.lock'), { recursive: true });

  for (const [name, prefix] of [['standalone source', ''], ['monorepo source', 'apps/ocean-surface/']]) {
    const fixture = deploymentFixture(name, prefix);
    // An installed script receives the Git root; invoking the source script
    // directly derives the component. Both must work from an unrelated cwd.
    const first = deployFixture(fixture, fixture.initialRevision, fixture.gitRoot);
    assert.equal(first.status, 0, `${first.stdout}\n${first.stderr}`);
    const release = join(fixture.state, 'releases', `${fixture.initialRevision}-proxy`, 'dist');
    assert.equal(readlinkSync(join(fixture.state, 'current')), `releases/${fixture.initialRevision}-proxy/dist`);
    assert.equal(readFileSync(join(release, 'index.html'), 'utf8'), '<p>first component build</p>');
    assert.equal(readFileSync(join(fixture.surface, 'dist/.deploy-sha'), 'utf8').trim(), fixture.initialRevision);
    assert.ok(existsSync(join(fixture.surface, 'extension/dist/ocean-surface-ui_bg.wasm')), 'extension sync belongs to the component');
    assert.ok(existsSync(join(fixture.surface, 'extension/dist/tokens.css')), 'extension receives stable CSS names');
    if (prefix) {
      assert.equal(existsSync(join(fixture.gitRoot, 'dist')), false, 'monorepo root must not receive Surface dist');
      assert.equal(existsSync(join(fixture.gitRoot, 'extension')), false, 'monorepo root must not receive Surface extension');
    }
    assert.deepEqual(readFileSync(fixture.commandLog, 'utf8').trim().split('\n'), [
      `cargo|${prefix}|test -p ocean-surface-proxy`,
      `cargo|${prefix}|clippy -p ocean-surface-proxy --all-targets -- -D warnings`,
      `cargo|${prefix}|check -p ocean-surface-ui --target wasm32-unknown-unknown`,
      `cargo|${prefix}|test -p ocean-surface-ui`,
      `cargo|${prefix}|clippy -p ocean-surface-ui --target wasm32-unknown-unknown -- -D warnings`,
      `cargo|${prefix}|fmt --all -- --check`,
      `node|${prefix}|scripts/surface-auto-deploy.test.mjs`,
      `trunk|${prefix}|build --release`,
      `cargo|${prefix}|build -p ocean-surface-proxy --release`,
    ], 'all build and contract gates must run in the Surface component');
    assert.equal(existsSync(join(fixture.state, 'tauri-rebuild-required')), false, 'initial deploy has no native predecessor');

    fixtureFile(join(fixture.surface, 'build-content'), '<p>next component build</p>');
    const frontendRevision = commitFixture(fixture, 'Frontend-only change');
    const frontend = deployFixture(fixture, frontendRevision);
    assert.equal(frontend.status, 0, `${frontend.stdout}\n${frontend.stderr}`);
    assert.equal(readFileSync(join(release, 'index.html'), 'utf8'), '<p>first component build</p>', 'outgoing release stays immutable');
    assert.equal(existsSync(join(fixture.state, 'tauri-rebuild-required')), false, 'frontend-only changes do not require a native rebuild');

    fixtureFile(join(fixture.surface, 'crates/ocean-tauri/src/lib.rs'), '// changed native shell\n');
    const nativeRevision = commitFixture(fixture, 'Native shell change');
    const native = deployFixture(fixture, nativeRevision, fixture.surface);
    assert.equal(native.status, 0, `${native.stdout}\n${native.stderr}`);
    assert.equal(readFileSync(join(fixture.state, 'tauri-rebuild-required'), 'utf8').trim(), nativeRevision, 'component native change requires a rebuild');
    assert.ok(native.stdout.includes(`${prefix}crates/ocean-tauri changed`));
    rmSync(join(fixture.state, 'tauri-rebuild-required'));

    const unrelatedShell = prefix ? 'crates/ocean-tauri/src/lib.rs' : 'other/crates/ocean-tauri/src/lib.rs';
    fixtureFile(join(fixture.gitRoot, unrelatedShell), '// unrelated project shell\n');
    const unrelatedRevision = commitFixture(fixture, 'Unrelated shell change');
    const unrelated = deployFixture(fixture, unrelatedRevision, fixture.gitRoot);
    assert.equal(unrelated.status, 0, `${unrelated.stdout}\n${unrelated.stderr}`);
    assert.equal(existsSync(join(fixture.state, 'tauri-rebuild-required')), false, 'other projects must not trigger the Surface native rebuild signal');

    fixtureFile(join(fixture.surface, 'build-content'), '<p>failed component build</p>');
    const failingRevision = commitFixture(fixture, 'Build failure candidate');
    const failedBuild = deployFixture(fixture, failingRevision, fixture.gitRoot, { FIXTURE_FAIL_BUILD: '1' });
    assert.notEqual(failedBuild.status, 0, 'a failed gate must stop promotion');
    assert.equal(readlinkSync(join(fixture.state, 'current')), `releases/${unrelatedRevision}-proxy/dist`, 'failed gates retain the last-good release');
    assert.equal(readFileSync(join(fixture.state, 'deployed-rev'), 'utf8').trim(), unrelatedRevision, 'failed gates retain the last-good marker');
    assert.equal(readFileSync(join(fixture.surface, 'dist/index.html'), 'utf8'), '<p>next component build</p>', 'failed gates must not sync a candidate bundle');
    assert.equal(existsSync(join(fixture.state, 'releases', `${failingRevision}-proxy`)), false, 'failed gates must not create a candidate release');
  }

  // TASK-87: a shell-source change must leave a VISIBLE rebuild marker.
  // A restart picks up new web assets but cannot pick up Rust changes, and two
  // native exec fixes once shipped "landed" while the installed app kept
  // running the old binary. Silence is the failure mode this pins against.
  assert.ok(
    railSrc.includes('tauri-rebuild-required'),
    'the rail must record an owed rebuild distinctly from a deferred restart',
  );
  assert.ok(
    railSrc.includes('crates/ocean-tauri'),
    'the rebuild signal must be scoped to shell-source changes, not every deploy',
  );
  assert.match(
    railSrc,
    /REBUILD REQUIRED/,
    'the operator-facing log line must say a rebuild is owed, not just that a restart was deferred',
  );

  // And the script the marker points at must exist, be executable, and refuse
  // the two things doing this by hand proved dangerous: clobbering a running
  // app, and trusting a successful build instead of checking the binary.
  const rebuildScript = join(repo, 'scripts', 'rebuild-tauri-app.sh');
  assert.ok(existsSync(rebuildScript), 'rebuild-tauri-app.sh must exist');
  const rebuildSrc = readFileSync(rebuildScript, 'utf8');
  assert.match(rebuildSrc, /pgrep -x "ocean-tauri"/, 'must refuse to replace a running app');
  assert.match(rebuildSrc, /strings "\$BIN"/, 'must verify guards in the built binary, not the exit code');
  assert.match(rebuildSrc, /rm -f "\$REBUILD_MARKER"/, 'must clear the marker it consumes');

  for (const [name, prefix] of [["installer standalone & <' paths", ''], ["installer monorepo & <' paths", 'apps/ocean-surface/']]) {
    const fixture = installerFixture(name, prefix);
    const installed = installFixture(fixture);
    assert.equal(installed.status, 0, `${installed.stdout}\n${installed.stderr}`);
    assert.equal(existsSync(fixture.launchLog), false, 'default installation never calls launchctl');
    assert.equal(readlinkSync(join(fixture.state, 'current')), `releases/${fixture.initialRevision}-proxy/dist`);
    assert.ok(existsSync(join(fixture.surface, 'dist/index.html')), 'installer retains in-place built bundle');
    assert.equal(readFileSync(join(fixture.surface, 'dist/.deploy-sha'), 'utf8').trim(), fixture.initialRevision);
    const authPath = join(fixture.state, 'proxy-auth.env');
    const authBefore = readFileSync(authPath);
    const selected = join(fixture.state, 'releases', `${fixture.initialRevision}-proxy`);
    const proxyBefore = readFileSync(join(selected, 'bin/ocean-surface-proxy'));
    fixtureFile(join(fixture.surface, 'target/release/ocean-surface-proxy'), '#!/usr/bin/env bash\necho wrong mutable executable\n');
    chmodSync(join(fixture.surface, 'target/release/ocean-surface-proxy'), 0o755);
    const launched = launchFixture(fixture);
    assert.equal(launched.status, 0, launched.stderr);
    assert.match(launched.stdout, new RegExp(`fixture proxy ${fixture.initialRevision}`), 'launcher executes selected immutable proxy bytes');
    assert.ok(launched.stdout.includes(`dist=${realpathSync(join(selected, 'dist'))}`), 'running proxy freezes the physical bundle paired with its executable');
    assert.deepEqual(readFileSync(join(selected, 'bin/ocean-surface-proxy')), proxyBefore);
    const health = JSON.parse(readFileSync(fixture.env.FIXTURE_HEALTH_FILE));
    assert.equal(health.release_revision, fixture.initialRevision);
    assert.equal(health.proxy_sha256, readFileSync(join(selected, 'proxy-sha256'), 'utf8').trim());
    writeFileSync(join(selected, 'bin/ocean-surface-proxy'), '#!/usr/bin/env bash\necho corrupt\n');
    chmodSync(join(selected, 'bin/ocean-surface-proxy'), 0o755);
    assert.notEqual(launchFixture(fixture).status, 0, 'a mismatched immutable proxy checksum fails before execution');
    writeFileSync(join(selected, 'bin/ocean-surface-proxy'), proxyBefore);
    chmodSync(join(selected, 'bin/ocean-surface-proxy'), 0o755);
    for (const suffix of ['proxy', 'auto-deploy']) {
      const path = join(fixture.env.OCEAN_SURFACE_OWNER_HOME, `Library/LaunchAgents/dev.risingtides.ocean-surface-${suffix}.plist`);
      const config = readPlist(path);
      const env = config.EnvironmentVariables;
      assert.deepEqual(config.ProgramArguments, [join(fixture.env.OCEAN_SURFACE_OWNER_HOME, `.config/ocean-surface/bin/ocean-surface-${suffix}.sh`)]);
      assert.equal(config.WorkingDirectory, realpathSync(fixture.surface));
      assert.equal(env.OCEAN_SURFACE_REPO, realpathSync(fixture.surface));
      assert.ok(env.PATH.includes(join(fixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin')));
      assert.equal(config.RunAtLoad, true);
      assert.equal(config.Label, `dev.risingtides.ocean-surface-${suffix}`);
      if (suffix === 'proxy') {
        assert.equal(env.OCEAN_SURFACE_DIST, fixture.state + '/current');
        assert.equal(config.KeepAlive, true);
        assert.equal(config.ThrottleInterval, 10);
      } else {
        assert.equal(env.OCEAN_SURFACE_STATE_DIR, fixture.state);
        assert.equal(env.CARGO_TARGET_DIR, join(realpathSync(fixture.surface), 'target'));
        assert.equal(config.StartInterval, 120);
        assert.equal(config.ThrottleInterval, 30);
      }
      const serialized = readFileSync(path, 'utf8');
      assert.equal(/__OCEAN_|fixture-secret|fixture-operator|OCEAN_SURFACE_AUTH|OCEAN_SURFACE_USER|OCEAN_SURFACE_PASS/.test(serialized), false, 'no unresolved paths or credentials in installed plists');
    }
    const snapshotAssets = clientAssets(fixture);
    const commandLogBeforeSnapshot = readFileSync(fixture.commandLog);
    const cpStub = join(fixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/cp');
    fixtureFile(cpStub, '#!/usr/bin/env bash\n[[ "$*" != *\.client-backup.* ]] || exit 13\nexec /bin/cp "$@"\n');
    chmodSync(cpStub, 0o755);
    const failedSnapshot = installFixture(fixture);
    assert.notEqual(failedSnapshot.status, 0, 'snapshot failure blocks before build/selection');
    assert.deepEqual(clientAssets(fixture), snapshotAssets, 'incomplete recovery inventory cannot remove existing client assets');
    assert.deepEqual(readFileSync(fixture.commandLog), commandLogBeforeSnapshot);
    assert.ok(existsSync(join(fixture.state, 'bin/ocean-surface-proxy.sh')));
    rmSync(cpStub);
    execFileSync('git', ['checkout', '-qb', 'feature'], { cwd: fixture.gitRoot });
    const guarded = installFixture(fixture);
    assert.notEqual(guarded.status, 0, 'non-main installation requires explicit override');
    assert.equal(existsSync(fixture.launchLog), false);
    const bootstrapped = installFixture(fixture, ['--allow-non-main', '--bootstrap']);
    assert.equal(bootstrapped.status, 0, `${bootstrapped.stdout}\n${bootstrapped.stderr}`);
    const launchCalls = readFileSync(fixture.launchLog, 'utf8').trim().split('\n');
    assert.equal(launchCalls.filter(line => line.startsWith('bootstrap ')).length, 2);
    assert.equal(launchCalls.filter(line => line.startsWith('bootout ')).length, 2);
    assert.equal(launchCalls.filter(line => /^(enable|disable) /.test(line)).length, 0, 'bootstrap preserves persistent operator overrides');
    assert.equal(launchCalls.filter(line => line.startsWith('kickstart ')).length, 1);
    assert.deepEqual(readFileSync(authPath), authBefore, 'install/recovery never rewrites owner auth');
    assert.equal(statSync(authPath).mode & 0o777, 0o600);

    // Same source marker must repair a missing listener and a healthy stale
    // process without rebuilding, and must compare paired provenance.
    const buildCommands = readFileSync(fixture.commandLog, 'utf8');
    for (const staleHealth of [null,
      { ok: true, service: 'ocean-surface-proxy' },
      { ok: true, service: 'ocean-surface-proxy', release_revision: 'older', proxy_sha256: health.proxy_sha256 },
      { ok: true, service: 'ocean-surface-proxy', release_revision: fixture.initialRevision, proxy_sha256: 'wrong-checksum' },
    ]) {
      if (staleHealth) writeFileSync(fixture.env.FIXTURE_HEALTH_FILE, JSON.stringify(staleHealth));
      else rmSync(fixture.env.FIXTURE_HEALTH_FILE);
      const repaired = deployFixture(fixture, fixture.initialRevision, fixture.surface, { OCEAN_SURFACE_NO_RESTART: '0' });
      assert.equal(repaired.status, 0, `${repaired.stdout}\n${repaired.stderr}`);
      assert.match(repaired.stdout, /CURRENT:/);
      assert.equal(JSON.parse(readFileSync(fixture.env.FIXTURE_HEALTH_FILE)).release_revision, fixture.initialRevision);
      assert.equal(readFileSync(fixture.commandLog, 'utf8'), buildCommands, 'same-revision listener repair reuses the immutable release');
    }
  }

  for (const [name, prefix] of [['legacy standalone', ''], ['legacy monorepo', 'apps/ocean-surface/']]) {
    const fixture = installerFixture(name, prefix);
    // Model migration from a different original checkout, not just rebuilding
    // the new component's mutable target.
    const { legacy, binary } = legacyFixture(fixture, join(root, `${name}-original`));
    const legacyBytes = readFileSync(binary);
    const legacyIndex = readFileSync(join(legacy, 'index.html'));
    const prior = clientAssets(fixture);
    const priorLaunchers = readFileSync(join(fixture.state, 'bin/ocean-surface-proxy.sh'));
    const plistPath = join(fixture.env.OCEAN_SURFACE_OWNER_HOME, 'Library/LaunchAgents/dev.risingtides.ocean-surface-proxy.plist');
    const priorPlist = readFileSync(plistPath);
    const failed = installFixture(fixture, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1' });
    assert.notEqual(failed.status, 0, 'failed candidate bootstrap rolls back rather than reporting deployment');
    assert.equal(readlinkSync(join(fixture.state, 'current')), 'releases/legacy-bundle');
    assert.equal(readFileSync(join(fixture.state, 'deployed-rev'), 'utf8').trim(), fixture.initialRevision);
    assert.deepEqual(clientAssets(fixture), prior);
    assert.deepEqual(readFileSync(join(fixture.state, 'bin/ocean-surface-proxy.sh')), priorLaunchers);
    assert.deepEqual(readFileSync(plistPath), priorPlist);
    assert.deepEqual(readFileSync(binary), legacyBytes, 'legacy recovery retains the original executable bytes');
    const recovery = realpathSync(join(fixture.state, 'legacy-recovery'));
    assert.equal(readFileSync(join(recovery, 'source-revision'), 'utf8').trim(), 'unknown', 'bundle marker cannot prove legacy executable provenance');
    assert.deepEqual(readFileSync(join(recovery, 'bin/ocean-surface-proxy')), legacyBytes);
    assert.deepEqual(readFileSync(join(legacy, 'index.html')), legacyIndex, 'legacy bundle bytes remain untouched');
    assert.equal(launchFixture(fixture).status, 0, 'restored legacy launcher remains usable');
    // The new launcher can also recover this old selection on an ordinary
    // restart without requiring a paired source revision to be invented.
    const newLegacyLaunch = spawnSync(join(fixture.surface, 'deploy/ocean-surface-proxy.sh'), [], { env: fixture.env, encoding: 'utf8' });
    assert.equal(newLegacyLaunch.status, 0, newLegacyLaunch.stderr);
    assert.match(newLegacyLaunch.stdout, /source=unknown/);
    assert.match(newLegacyLaunch.stdout, /original legacy proxy/);
    const priorCalls = readFileSync(fixture.launchLog);
    const migrated = installFixture(fixture);
    assert.equal(migrated.status, 0, `${migrated.stdout}\n${migrated.stderr}`);
    assert.equal(readlinkSync(join(fixture.state, 'current')), `releases/${fixture.initialRevision}-proxy/dist`, 'same-revision legacy migration stages a paired release');
    assert.deepEqual(readFileSync(join(recovery, 'bin/ocean-surface-proxy')), legacyBytes);
    assert.deepEqual(readFileSync(fixture.launchLog), priorCalls, 'stage-only migration makes no live supervision calls');
  }

  const missingLegacy = installerFixture('missing legacy binary', 'apps/ocean-surface/');
  const missing = legacyFixture(missingLegacy);
  rmSync(missing.binary);
  const blockedMigration = installFixture(missingLegacy);
  assert.notEqual(blockedMigration.status, 0);
  assert.match(blockedMigration.stderr, /cannot recover the legacy proxy/);
  assert.equal(existsSync(missingLegacy.commandLog), false, 'missing recovery bytes block before a mutable build');
  assert.equal(readlinkSync(join(missingLegacy.state, 'current')), 'releases/legacy-bundle');

  for (const [proxyLoaded, autoLoaded] of [[false, false], [true, false], [false, true], [true, true]]) {
    const fixture = installerFixture(`recover loaded ${proxyLoaded}-${autoLoaded}`, 'apps/ocean-surface/');
    legacyFixture(fixture);
    supervision(fixture, proxyLoaded, autoLoaded);
    const failed = installFixture(fixture, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1' });
    assert.equal(failed.status, 14, `${failed.stdout}\n${failed.stderr}`);
    const states = JSON.parse(readFileSync(fixture.supervisionState));
    assert.deepEqual(states[proxyLabel], { loaded: proxyLoaded, disabled: false }, 'restore exact prior proxy loaded/cold state');
    assert.deepEqual(states[autoLabel], { loaded: autoLoaded, disabled: false }, 'restore exact prior deploy watcher loaded/cold state');
    assert.equal(/^(enable|disable) /m.test(readFileSync(fixture.launchLog, 'utf8')), false);
  }
  for (const label of [proxyLabel, autoLabel]) {
    for (const override of [true, 'invalid', 'true trailing-invalid']) {
      const fixture = installerFixture(`disabled ${label}-${override}`, '');
      const legacy = legacyFixture(fixture);
      supervision(fixture, true, true, label, override);
      const oldStates = readFileSync(fixture.supervisionState);
      const oldBytes = readFileSync(legacy.binary);
      const oldPlist = readFileSync(join(fixture.env.OCEAN_SURFACE_OWNER_HOME, `Library/LaunchAgents/${label}.plist`));
      const rejected = installFixture(fixture, ['--bootstrap']);
      assert.notEqual(rejected.status, 0, 'true or malformed named disabled override blocks bootstrap');
      assert.equal(readlinkSync(join(fixture.state, 'current')), 'releases/legacy-bundle');
      assert.deepEqual(readFileSync(legacy.binary), oldBytes);
      assert.deepEqual(readFileSync(fixture.supervisionState), oldStates);
      assert.deepEqual(readFileSync(join(fixture.env.OCEAN_SURFACE_OWNER_HOME, `Library/LaunchAgents/${label}.plist`)), oldPlist);
      assert.equal(/^(bootout|bootstrap|kickstart|enable|disable) /m.test(readFileSync(fixture.launchLog, 'utf8')), false, 'preflight must not mutate loaded jobs or their overrides');
    }
  }

  for (const mode of ['dirty', 'untracked', 'head']) {
    const fixture = installerFixture(`source changes ${mode}`, 'apps/ocean-surface/');
    legacyFixture(fixture);
    const prior = clientAssets(fixture);
    const rejected = installFixture(fixture, [], { FIXTURE_SOURCE_CHANGE: mode });
    assert.notEqual(rejected.status, 0, 'changing source during build must not publish a revision');
    assert.equal(readlinkSync(join(fixture.state, 'current')), 'releases/legacy-bundle');
    assert.deepEqual(clientAssets(fixture), prior);
    assert.equal(existsSync(join(fixture.state, 'releases', `${fixture.initialRevision}-proxy`)), false);
  }
  const gitFailure = installerFixture('git status failure', '');
  const gitStub = join(gitFailure.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/git');
  fixtureFile(gitStub, '#!/usr/bin/env bash\n[[ "$*" != *status* ]] || exit 19\nexec /usr/bin/git "$@"\n');
  chmodSync(gitStub, 0o755);
  const rejectedStatus = installFixture(gitFailure);
  assert.notEqual(rejectedStatus.status, 0, 'git failure cannot be interpreted as clean source');
  assert.equal(existsSync(gitFailure.commandLog), false);

  const recoveryFailure = installerFixture('client restoration failure', 'apps/ocean-surface/');
  legacyFixture(recoveryFailure);
  supervision(recoveryFailure, true, true);
  const oldAssets = clientAssets(recoveryFailure);
  const mvStub = join(recoveryFailure.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/mv');
  fixtureFile(mvStub, '#!/usr/bin/env bash\n[[ "$*" != *\.client-backup.* ]] || exit 16\nexec /bin/mv "$@"\n');
  chmodSync(mvStub, 0o755);
  const touchStub = join(recoveryFailure.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/touch');
  fixtureFile(touchStub, '#!/usr/bin/env bash\nif [[ -f "$FIXTURE_FAILURE_FILE" && "$*" == *recovery-* ]]; then exit 28; fi\nexec /usr/bin/touch "$@"\n');
  chmodSync(touchStub, 0o755);
  const unproven = installFixture(recoveryFailure, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1' });
  assert.equal(unproven.status, 14);
  const backup = retainedBackups(recoveryFailure.state)[0];
  assert.ok(backup, 'failed restoration retains the preallocated recovery evidence');
  assert.equal(statSync(backup).mode & 0o777, 0o700);
  assert.equal(readFileSync(join(backup, 'dist/index.html')).toString('hex'), oldAssets['dist/index.html']);
  assert.match(unproven.stderr, /retained owner-only backup|preserve owner-only backup/);

  const copyFailure = installerFixture('legacy copy failure', '');
  const oldProxy = legacyFixture(copyFailure);
  supervision(copyFailure, true, true);
  const cpStub = join(copyFailure.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/cp');
  fixtureFile(cpStub, `#!/usr/bin/env bash
if [[ "$*" == *\.proxy-recovery.* ]]; then
  printf 'partial corrupt executable' > "\${!#}"
  exit 29
fi
exec /bin/cp "$@"
`);
  chmodSync(cpStub, 0o755);
  const copiedPartially = installFixture(copyFailure, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1' });
  assert.equal(copiedPartially.status, 14);
  assert.notEqual(readFileSync(oldProxy.binary, 'utf8'), 'partial corrupt executable', 'failed copy must never rename partial bytes over the original path');
  assert.ok(retainedBackups(copyFailure.state).length);
  assert.match(copiedPartially.stderr, /prior legacy executable restoration incomplete/);

  const reloadFailure = installerFixture('recovery supervision failure', '');
  legacyFixture(reloadFailure);
  supervision(reloadFailure, true, true);
  const notReloaded = installFixture(reloadFailure, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1', FIXTURE_FAIL_RECOVERY_BOOTSTRAP: '1' });
  assert.equal(notReloaded.status, 14);
  assert.ok(retainedBackups(reloadFailure.state).length, 'failed supervision proof retains evidence even after successful asset restore');
  assert.match(notReloaded.stderr, /prior supervision\/listening unverified/);

  const invalidCapture = installerFixture('invalid captured checksum during cleanup', '');
  legacyFixture(invalidCapture);
  supervision(invalidCapture, true, true);
  const invalidRecovery = installFixture(invalidCapture, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1', FIXTURE_CORRUPT_CAPTURE: '1' });
  assert.equal(invalidRecovery.status, 14, 'fatal captured-release validation must not exit the cleanup process');
  assert.equal(existsSync(join(invalidCapture.state, 'auto-deploy.lock')), false);
  assert.ok(retainedBackups(invalidCapture.state).length);
  assert.match(invalidRecovery.stderr, /prior legacy executable restoration incomplete/);

  const invalidHealth = installerFixture('invalid prior checksum during recovery health', 'apps/ocean-surface/');
  const healthInstalled = installFixture(invalidHealth, ['--bootstrap']);
  assert.equal(healthInstalled.status, 0, healthInstalled.stderr);
  const priorChecksum = join(invalidHealth.state, 'releases', `${invalidHealth.initialRevision}-proxy`, 'proxy-sha256');
  fixtureFile(join(invalidHealth.surface, 'build-content'), '<p>new health candidate</p>');
  commitFixture(invalidHealth, 'New recovery-health candidate');
  const invalidHealthRecovery = installFixture(invalidHealth, ['--bootstrap'], { FIXTURE_FAIL_RESTART_ONCE: '1', FIXTURE_CORRUPT_PREVIOUS_PROXY: priorChecksum, FIXTURE_SKIP_PROXY_EXEC: '1' });
  assert.equal(invalidHealthRecovery.status, 14, 'fatal health resolution must remain a bounded failed health result');
  assert.equal(existsSync(join(invalidHealth.state, 'auto-deploy.lock')), false);
  assert.ok(retainedBackups(invalidHealth.state).length);
  assert.match(invalidHealthRecovery.stderr, /prior supervision\/listening unverified/);

  // The direct promotion child also owns recovery evidence and cannot discard
  // bytes after a partial client restore, or accept a still-selected candidate
  // as proof that the prior selection was recovered.
  const cliBefore = new Set(retainedBackups(state));
  const cliMv = join(promotionFixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/mv');
  fixtureFile(cliMv, '#!/usr/bin/env bash\n[[ "$*" != *\.client-backup.* ]] || exit 16\nexec /bin/mv "$@"\n');
  chmodSync(cliMv, 0o755);
  fixtureFile(fixtureRsync, `#!/usr/bin/env bash
if [[ "$*" == *${shellQuote(`${promotionFixture.surface}/dist/`)}* ]]; then
  printf 'failed partial client sync' > ${shellQuote(join(promotionFixture.surface, 'dist/index.html'))}
  exit 13
fi
exec /usr/bin/rsync "$@"
`);
  chmodSync(fixtureRsync, 0o755);
  const cliUnproven = spawnSync(script, ['--promote', staged, 'cli-restore-failed'], { env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state }, encoding: 'utf8' });
  assert.equal(cliUnproven.status, 13);
  const cliBackup = retainedBackups(state).find(path => !cliBefore.has(path));
  assert.ok(cliBackup, 'direct promotion retains its own backup after failed restoration');
  assert.ok(existsSync(join(cliBackup, 'dist/index.html')));
  assert.equal(statSync(cliBackup).mode & 0o777, 0o700);
  rmSync(cliMv);
  rmSync(fixtureRsync);
  execFileSync('/bin/mv', [join(cliBackup, 'dist'), join(promotionFixture.surface, 'dist')]);
  const lnStub = join(promotionFixture.env.OCEAN_SURFACE_OWNER_HOME, '.cargo/bin/ln');
  fixtureFile(lnStub, '#!/usr/bin/env bash\n[[ "$2" != "releases/in-place-proxy/dist" ]] || exit 15\nexec /bin/ln "$@"\n');
  chmodSync(lnStub, 0o755);
  fixtureFile(fixtureLaunchctl, '#!/usr/bin/env bash\nexit 14\n');
  chmodSync(fixtureLaunchctl, 0o755);
  const notSelectedBack = spawnSync(script, ['--promote', staged, 'selection-restore-failed'], { env: { ...promotionEnv, OCEAN_SURFACE_STATE_DIR: state, OCEAN_SURFACE_NO_RESTART: '0' }, encoding: 'utf8' });
  assert.equal(notSelectedBack.status, 14);
  assert.match(notSelectedBack.stderr, /prior selection restoration incomplete/);
  assert.match(notSelectedBack.stderr, /supervision recovery skipped/);
  assert.equal(readlinkSync(join(state, 'current')), 'releases/selection-restore-failed-proxy/dist');
  assert.equal(readFileSync(join(state, 'deployed-rev'), 'utf8').trim(), 'in-place');
  assert.equal(notSelectedBack.stdout.includes('DEPLOYED:'), false);
  assert.ok(retainedBackups(state).length);

  console.log('ALL PASS: Surface atomic promotion and standalone/monorepo deployment contracts');
} finally {
  rmSync(root, { recursive: true, force: true });
}
