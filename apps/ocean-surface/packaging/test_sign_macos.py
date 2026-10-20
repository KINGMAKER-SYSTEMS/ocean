import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import sign_macos as signing

REVISION = 'a' * 40
TEAM = 'A123456789'
IDENTITY = 'B' * 40


class SigningTests(unittest.TestCase):
    def fixture(self, root):
        image = root / 'Ocean.dmg'
        image.write_bytes(b'fixture')
        receipt = {'revision': REVISION, 'target': 'aarch64-apple-darwin', 'signing': 'unsigned',
                   'files': {image.name: signing.desktop.digest(image)},
                   'daemon_sha256': 'c' * 64, 'mcp_sha256': 'd' * 64}
        (root / 'candidate.json').write_text(json.dumps(receipt))
        return receipt, image

    def test_receipt_pins_source_and_image_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            receipt, image = self.fixture(root)
            self.assertEqual(signing.read_candidate(root, REVISION), (receipt, image.resolve()))
            with self.assertRaises(RuntimeError):
                signing.read_candidate(root, 'e' * 40)
            image.write_bytes(b'changed')
            with self.assertRaises(RuntimeError):
                signing.read_candidate(root, REVISION)

    def test_untrusted_receipts_cannot_select_other_paths_or_platforms(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            receipt, image = self.fixture(root)
            variants = [[], dict(receipt, target='x86_64-pc-windows-msvc'),
                        dict(receipt, signing='signed'), dict(receipt, files={}),
                        dict(receipt, files={'../Ocean.dmg': signing.desktop.digest(image)}),
                        dict(receipt, files={'/Ocean.dmg': signing.desktop.digest(image)}),
                        dict(receipt, files={'sub\\Ocean.dmg': signing.desktop.digest(image)}),
                        dict(receipt, mcp_sha256=None)]
            for bad in variants:
                with self.subTest(receipt=bad):
                    (root / 'candidate.json').write_text(json.dumps(bad))
                    with self.assertRaises(RuntimeError):
                        signing.read_candidate(root, REVISION)

    @unittest.skipUnless(os.name == "posix", "Mac signing link checks require POSIX link support")
    def test_symlinked_artifacts_and_receipts_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            _, image = self.fixture(root)
            saved = root / 'saved.dmg'
            image.rename(saved)
            image.symlink_to(saved)
            with self.assertRaises(RuntimeError):
                signing.read_candidate(root, REVISION)
            image.unlink()
            saved.rename(image)
            manifest = root / 'candidate.json'
            saved = root / 'saved.json'
            manifest.rename(saved)
            manifest.symlink_to(saved)
            with self.assertRaises(RuntimeError):
                signing.read_candidate(root, REVISION)

    def test_existing_outputs_fail_before_any_signing_command(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            output = root / 'published'
            output.mkdir()
            (output / 'keep').write_text('original')
            with patch.object(signing.platform, 'system', return_value='Darwin'), patch.object(signing, 'command') as command:
                with self.assertRaises(RuntimeError):
                    signing.prepare(root, REVISION, IDENTITY, TEAM, output)
                command.assert_not_called()
            self.assertEqual((output / 'keep').read_text(), 'original')

    def test_unexpected_team_adhoc_missing_runtime_or_timestamp_never_pass(self):
        good = f'CodeDirectory v=20500 size=900 flags=0x10000(runtime) hashes=100\nAuthority=Developer ID Application: Fixture ({TEAM})\nTeamIdentifier={TEAM}\nTimestamp=Oct 20, 2026 at 12:00:00\n'
        self.assertTrue(signing.signature_metadata(good, TEAM)['hardened_runtime'])
        for bad in (good.replace(TEAM, 'Z123456789'), good.replace('Developer ID Application:', 'Apple Development:'),
                    good.replace('flags=0x10000(runtime)', 'flags=0x2(adhoc) runtime metadata'), good.replace('Timestamp=', 'Signed Time='),
                    good.replace('Oct 20, 2026 at 12:00:00', 'none')):
            with self.assertRaises(RuntimeError):
                signing.signature_metadata(bad, TEAM)

    def test_keychain_failure_is_actionable_without_repair_or_retry(self):
        reply = subprocess.CompletedProcess(['codesign'], 1, '', 'errSecInternalComponent private diagnostic')
        with patch.object(signing.subprocess, 'run', return_value=reply) as run:
            with self.assertRaisesRegex(RuntimeError, 'Check certificate trust and Keychain access locally') as caught:
                signing.command(['/usr/bin/codesign', 'fixture'])
            self.assertNotIn('private diagnostic', str(caught.exception))
            self.assertEqual(run.call_count, 1)


if __name__ == '__main__':
    unittest.main()
