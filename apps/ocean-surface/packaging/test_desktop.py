import io
from pathlib import Path
import struct
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import build_desktop as desktop


class DesktopArtifactTests(unittest.TestCase):
    def test_unbundled_windows_runtime_is_refused(self):
        system = 'Image has the following dependencies:\n    KERNEL32.dll\n    bcrypt.dll\n    msvcrt.dll\n'
        desktop.verify_windows_runtime(system)
        for library in ('VCRUNTIME140.dll', 'vcruntime140_1.dll', 'MSVCP140.dll', 'MSVCR120.dll', 'CONCRT140.dll'):
            with self.assertRaises(RuntimeError):
                desktop.verify_windows_runtime(system + f'    {library}\n')
        with self.assertRaises(RuntimeError):
            desktop.verify_windows_runtime('invalid executable')

    def test_busy_owned_image_is_retried_before_force(self):
        busy = subprocess.CalledProcessError(16, 'hdiutil')
        with patch.object(desktop, 'run', side_effect=[busy, None]) as run, patch.object(desktop.time, 'sleep'):
            desktop.detach_image(Path('/owned/mount'))
            self.assertEqual(run.call_count, 2)
            self.assertTrue(all('-force' not in call.args[0] for call in run.call_args_list))
        with patch.object(desktop, 'run', side_effect=[busy, busy, busy, None]) as run, patch.object(desktop.time, 'sleep'):
            desktop.detach_image(Path('/owned/mount'))
            self.assertEqual(run.call_args.args[0], ['hdiutil', 'detach', '-force', Path('/owned/mount')])

    def test_developer_machine_libraries_are_refused(self):
        system = "Ocean:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)\n"
        desktop.verify_macos_libraries(system)
        for path in ('/opt/homebrew/lib/libssl.dylib', '/usr/local/lib/libcrypto.dylib',
                     '@rpath/libssl.dylib', '/Users/developer/lib.dylib'):
            with self.assertRaises(RuntimeError):
                desktop.verify_macos_libraries(system + f'\t{path} (compatibility version 1.0.0)\n')
        with self.assertRaises(RuntimeError):
            desktop.verify_macos_libraries('Ocean:\n')

    def test_archive_paths_never_control_extraction_destination(self):
        payload = io.BytesIO()
        with zipfile.ZipFile(payload, 'w') as archive:
            archive.writestr('../../cargo-tauri', b'fixture')
        self.assertEqual(desktop.unpack_tool(payload.getvalue(), 'cargo-tauri', True), b'fixture')
        with zipfile.ZipFile(payload, 'a') as archive:
            archive.writestr('other/cargo-tauri', b'ambiguous')
        with self.assertRaises(RuntimeError):
            desktop.unpack_tool(payload.getvalue(), 'cargo-tauri', True)
        payload = io.BytesIO()
        with tarfile.open(fileobj=payload, mode='w:gz') as archive:
            item = tarfile.TarInfo('cargo-about')
            item.type = tarfile.SYMTYPE
            item.linkname = '/outside'
            archive.addfile(item)
        with self.assertRaises(RuntimeError):
            desktop.unpack_tool(payload.getvalue(), 'cargo-about', False)

    def test_wrong_architecture_and_console_release_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fixture'
            data = bytearray(222)
            data[:2] = b'MZ'
            struct.pack_into('<I', data, 60, 128)
            data[128:132] = b'PE\0\0'
            struct.pack_into('<H', data, 132, 0x8664)
            struct.pack_into('<H', data, 220, 2)
            path.write_bytes(data)
            desktop.binary_architecture(path, 'x86_64-pc-windows-msvc', gui=True)
            struct.pack_into('<H', data, 220, 3)
            path.write_bytes(data)
            with self.assertRaises(RuntimeError):
                desktop.binary_architecture(path, 'x86_64-pc-windows-msvc', gui=True)
            path.write_bytes(b'\xcf\xfa\xed\xfe' + struct.pack('<I', 0x0100000c))
            desktop.binary_architecture(path, 'aarch64-apple-darwin')
            with self.assertRaises(RuntimeError):
                desktop.binary_architecture(path, 'x86_64-apple-darwin')

    def test_corrupt_and_truncated_web_modules_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'index.html').write_text('fixture')
            for content in (b'', b'\0' * 100, b'\0asm\x01\0\0\0'):
                (root / 'app.wasm').write_bytes(content)
                with self.assertRaises(RuntimeError):
                    desktop.verify_web_bundle(root)
            (root / 'app.wasm').write_bytes(b'\0asm\x01\0\0\0fixture')
            desktop.verify_web_bundle(root)

    def test_stale_daemon_and_missing_notices_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            app = root / 'Ocean.app'
            bins = app / 'Contents/MacOS'
            bins.mkdir(parents=True)
            header = b'\xcf\xfa\xed\xfe' + struct.pack('<I', 0x0100000c)
            daemon = root / 'daemon'
            daemon.write_bytes(header + b'new')
            (bins / 'ocean-tauri').write_bytes(header)
            (bins / 'ocean-daemon').write_bytes(header + b'old')
            legal = root / 'legal'
            legal.mkdir()
            (legal / 'NOTICE.txt').write_text('required notice')
            with self.assertRaisesRegex(RuntimeError, 'does not match'):
                desktop.verify_payload(app, daemon, legal, 'aarch64-apple-darwin')
            (bins / 'ocean-daemon').write_bytes(daemon.read_bytes())
            with self.assertRaises(FileNotFoundError):
                desktop.verify_payload(app, daemon, legal, 'aarch64-apple-darwin')
            dest = app / 'Contents/Resources/legal'
            dest.mkdir(parents=True)
            (dest / 'NOTICE.txt').write_text('required notice')
            desktop.verify_payload(app, daemon, legal, 'aarch64-apple-darwin')


if __name__ == '__main__':
    unittest.main()
