#!/usr/bin/env python3
"""Exercise the real NSIS installer only on a disposable hosted Windows runner."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def snapshot(root):
    return {p.relative_to(root).as_posix(): digest(p) for p in root.rglob('*') if p.is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('revision')
    args = parser.parse_args()
    if os.name != 'nt' or os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('Installer acceptance requires a disposable GitHub-hosted Windows runner')
    import winreg
    uninstall_key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\Ocean'
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, uninstall_key):
            raise RuntimeError('An existing Ocean installation must not be modified')
    except FileNotFoundError:
        pass
    processes = subprocess.check_output(['tasklist', '/FI', 'IMAGENAME eq ocean-tauri.exe', '/FO', 'CSV', '/NH'], text=True)
    if 'ocean-tauri.exe' in processes.lower():
        raise RuntimeError('An existing Ocean process must not be modified')
    receipt = json.loads((args.candidate / 'candidate.json').read_text())
    assert receipt['revision'] == args.revision
    assert receipt['target'] == 'x86_64-pc-windows-msvc'
    assert len(receipt['files']) == 1
    name, checksum = next(iter(receipt['files'].items()))
    assert Path(name).name == name and name.lower().endswith('.exe')
    installer = (args.candidate / name).resolve(strict=True)
    assert digest(installer) == checksum
    appdata = Path(os.environ['APPDATA']) / 'dev.ocean.surface'
    home = Path(os.environ.get('HOME', ''))
    if not home.is_absolute():
        home = Path(os.environ['USERPROFILE'])
    profile = home / '.config/ocean-rs'
    if appdata.exists() or profile.exists():
        raise RuntimeError('An existing Ocean profile must not be modified')
    # Registry and desktop state are confined to the disposable hosted account.
    with tempfile.TemporaryDirectory(prefix='ocean-installer-') as directory:
        root = Path(directory)
        installed = root / 'app with spaces'
        appdata.mkdir()
        marker = appdata / 'installer-acceptance'
        marker.write_bytes(b'preserve desktop preferences across updates')
        profile.mkdir(parents=True)
        (profile / 'retained-state').write_bytes(b'preserve daemon state across updates')
        expected_profile = snapshot(profile)
        child = None
        handle = None
        removed = False
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p,
                                      ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        uninstaller = installed / 'uninstall.exe'

        def install():
            # NSIS requires /D to be last and unquoted, including spaces. This
            # is a CreateProcess command line, never a shell command.
            command = subprocess.list2cmdline([str(installer), '/S']) + ' /D=' + str(installed)
            return subprocess.run(command, timeout=180, check=False).returncode

        def uninstall():
            # _?= prevents asynchronous self-copy so wait observes completion.
            command = subprocess.list2cmdline([str(uninstaller), '/S']) + ' _?=' + str(installed)
            return subprocess.run(command, timeout=180, check=False).returncode

        def preserved():
            assert marker.read_bytes() == b'preserve desktop preferences across updates'
            assert snapshot(profile) == expected_profile

        try:
            assert install() == 0, 'fresh NSIS installation failed'
            for binary, field in [('ocean-daemon.exe', 'daemon_sha256'), ('ocean-mcp.exe', 'mcp_sha256')]:
                assert digest(installed / binary) == receipt[field]
            original_app = digest(installed / 'ocean-tauri.exe')
            # Make the existing app distinguishable from the incoming payload;
            # equal-version byte hashes alone could hide a partial replacement.
            with (installed / 'ocean-tauri.exe').open('ab') as stream:
                stream.write(b'\nexisting-installation-fixture\n')
            before = snapshot(installed)
            # A real process with the app's basename exercises the template's
            # process-check branch without opening an interactive GUI. Use only
            # a runner-owned copy of Windows' loopback ping executable.
            helper = root / 'ocean-tauri.exe'
            shutil.copy2(Path(os.environ['SYSTEMROOT']) / 'System32/ping.exe', helper)
            child = subprocess.Popen([str(helper), '-n', '600', '127.0.0.1'], stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW)
            time.sleep(.5)
            assert child.poll() is None, 'process fixture did not stay alive'
            for operation in (install, uninstall):
                assert operation() != 0, 'installer must refuse a running Ocean process'
                assert child.poll() is None, 'installer force-terminated the process'
                assert snapshot(installed) == before, 'refused operation changed installed files'
                preserved()
            child.terminate()
            child.wait(timeout=10)
            child = None
            # A busy sidecar must also stop before replacing the main app.
            for binary in ('ocean-daemon.exe', 'ocean-mcp.exe'):
                handle = kernel.CreateFileW(str(installed / binary), 0x80000000, 0, None, 3, 128, None)
                assert handle != ctypes.c_void_p(-1).value, ctypes.get_last_error()
                for operation in (install, uninstall):
                    assert operation() != 0, f'installer ignored the locked {binary}'
                    # The lock denies reads too; release before snapshotting.
                    preserved()
                kernel.CloseHandle(handle)
                handle = None
                assert snapshot(installed) == before, 'busy sidecar refusal changed installed files'
            assert install() == 0, 'idle reinstall failed'
            preserved()
            for binary, field in [('ocean-daemon.exe', 'daemon_sha256'), ('ocean-mcp.exe', 'mcp_sha256')]:
                assert digest(installed / binary) == receipt[field]
            assert digest(installed / 'ocean-tauri.exe') == original_app
            assert uninstall() == 0, 'idle uninstall failed'
            removed = True
            assert all(not (installed / binary).exists() for binary in ('ocean-tauri.exe', 'ocean-daemon.exe', 'ocean-mcp.exe'))
            preserved()
            print('PASS: actual NSIS install, process/sidecar refusal without mutation, idle reinstall/uninstall, retained profile data')
        finally:
            if handle is not None and handle != ctypes.c_void_p(-1).value:
                kernel.CloseHandle(handle)
            if child is not None:
                child.kill()
                child.wait(timeout=10)
            if not removed and uninstaller.is_file():
                uninstall()
            # Remove only the fixture marker/directory we exclusively created.
            marker.unlink(missing_ok=True)
            appdata.rmdir()
            (profile / 'retained-state').unlink(missing_ok=True)
            profile.rmdir()


if __name__ == '__main__':
    main()
