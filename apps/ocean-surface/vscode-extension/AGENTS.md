# Ocean editor extension

## Purpose

VS Code and Cursor ACP client, including editor-backed file operations and local terminals.

## Ownership

- `src/handlers/` owns ACP editor I/O; the Surface parent owns shared UI direction.
- `tests/handlers.test.cjs` executes production handlers with disposable editor/provider seams.

## Local Contracts

- An open-document write succeeds only after `TextDocument.save()` returns true. Save refusal must not emit a persisted-edit receipt.
- Capture existing file contents before overwriting. Only a known `FileNotFound` permits a new-file receipt; other read failures stop before writing.
- Terminal `command` and `args` retain literal argument-vector semantics. Shell interpretation requires an explicitly selected shell command.
- Decode stdout and stderr independently across UTF-8 chunk boundaries, flush each at stream end, and retain the byte-bounded capture.

## Work Guidance

- Keep runtime/provider authority in the daemon and preserve the parent's minimal editor UI.

## Verification

- `npm test` — production handler regressions with editor mocks and a disposable literal-argv child.
- `npm run lint` — TypeScript checking.
- `npm run package` — actual extension bundle.

## Child devlog Index

- None.
