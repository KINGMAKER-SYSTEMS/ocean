# Mac signing handoff

`build_desktop.py` produces unsigned candidates. `sign_macos.py` prepares a
separate signed app and ZIP from an existing candidate; it does not install,
notarize, publish, export a key, or change Keychain permissions.

Use a reviewed source revision and its verified GitHub artifact. Keep the
original DMG and `candidate.json` together. Do not substitute an older passing
installer for a newer application revision.

From the Surface component:

```sh
python3 packaging/sign_macos.py /path/to/candidate \
  --revision FULL_SOURCE_SHA \
  --identity CERTIFICATE_SHA1 \
  --team-id APPLE_TEAM_ID \
  --output target/signed-desktop/UNIQUE_DIRECTORY
```

Find the certificate identifier with `security find-identity -v -p codesigning`.
The identity must be a Developer ID Application certificate. Keep passwords,
private keys and notarization credentials out of the repository and command
arguments. Resolve any Keychain interaction denial locally; the tool does not
unlock a keychain or change its access controls.

The tool verifies the original receipt and disk-image hash, works on a verified
private snapshot, checks both bundled sidecar hashes and all three executable
architectures/system libraries, and refuses unexpected native code or links.
It signs the sidecars before the app, verifies the expected team, hardened
runtime and secure timestamps, and publishes the output directory only after
verification. Existing output directories are never reused. The receipt records
`notarization: not_submitted`; signing is not distribution acceptance. Signing
an Intel artifact on Apple Silicon does not prove Intel execution.

## Notarization and acceptance

Use an existing local `notarytool` Keychain profile. Submit the prepared ZIP with
`xcrun notarytool submit`, retaining its submission id and response. A timeout or
lost response is not a rejection: inspect that submission with `info`, `wait`
and `log` before submitting anything again. Require Apple's Accepted status and
verify that the notarization log's SHA-256 names the submitted ZIP.

Staple and validate the ticket on `Ocean.app`, then run strict deep codesign
verification and Gatekeeper execute assessment. The submitted ZIP cannot receive
a stapled ticket; create a new distribution archive containing the stapled app
and record its new hash. If distributing a DMG, sign, notarize and staple that
final disk image too. Retain the original submitted bytes and submission records.

Follow [Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
and [distribution signing requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
Tauri's [Mac signing guide](https://v2.tauri.app/distribute/sign/macos/) describes
the supported certificate types and signing configuration.

Gatekeeper acceptance, clean-machine installation, provider sign-in/reply,
shared-room acceptance and update/recovery are separate release checks. Do not
publish a local signature-only proof as a completed coworker release.
