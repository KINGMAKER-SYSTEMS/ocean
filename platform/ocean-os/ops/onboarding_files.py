#!/usr/bin/env python3
"""Owner-only onboarding files; Python 3.11+ for the daemon's strict TOML shape."""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import tomllib
import unicodedata

SCOPE = "@risingtides-dev:registry=https://npm.pkg.github.com"
TOKEN_KEY = "//npm.pkg.github.com/:_authToken="


def checked_path(raw):
    path = Path(raw)
    if not path.is_absolute():
        raise ValueError("configuration paths must be absolute")
    for part in [*reversed(path.parents), path]:
        if part.is_symlink():
            raise ValueError("symlink configuration paths are refused")
    return path


def owned_file(path):
    if not path.exists():
        return
    meta = path.stat(follow_symlinks=False)
    if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1:
        raise ValueError("configuration destination must be an owned regular file with one link")


def owned_directory(path):
    if path.exists():
        meta = path.stat(follow_symlinks=False)
        if not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.getuid():
            raise ValueError("configuration directory must be owned by this user")


def valid_member(value):
    return isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9._@-]+", value) is not None


def identity_file(args):
    if not valid_member(args.member):
        raise ValueError("member must be a nonempty ASCII identifier: letters, digits, . _ @ -")
    if len(args.display_name) > 80 or any(unicodedata.category(c) == "Cc" for c in args.display_name):
        raise ValueError("display name must be at most 80 characters without controls")
    directory = checked_path(args.config_dir)
    owned_directory(directory)
    path = checked_path(str(directory / "member.toml"))
    owned_file(path)
    if path.exists() and not args.force:
        try:
            current = tomllib.loads(path.read_text())
            valid = set(current) <= {"member_id", "display_name"} and valid_member(current.get("member_id"))
            valid = valid and ("display_name" not in current or isinstance(current["display_name"], str))
        except (ValueError, UnicodeError):
            valid = False
        if not valid:
            raise ValueError("existing member.toml is malformed; inspect it or use --force")
        if current["member_id"] != args.member:
            raise ValueError("existing member.toml names another member; use --force only for an intentional replacement")
    return path


def npmrc_file(args):
    path = checked_path(args.npmrc)
    owned_directory(path.parent)
    owned_file(path)
    return path


def atomic_write(path, text, private_directory=False):
    # Recheck immediately before mutation; replacements never follow a destination link.
    checked_path(str(path))
    owned_file(path)
    owned_directory(path.parent)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if private_directory:
        path.parent.chmod(0o700)
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(text)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["validate", "identity", "npmrc"])
    parser.add_argument("--config-dir")
    parser.add_argument("--member", default="")
    parser.add_argument("--display-name", default="")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--npmrc")
    args = parser.parse_args()
    if args.action in {"validate", "identity"}:
        path = identity_file(args)
        if args.action == "identity":
            text = f"member_id = {json.dumps(args.member, ensure_ascii=False)}\n"
            if args.display_name:
                # JSON basic-string escapes are valid TOML for these control-free values.
                text += f"display_name = {json.dumps(args.display_name, ensure_ascii=False)}\n"
            atomic_write(path, text, private_directory=True)
    if args.action in {"validate", "npmrc"}:
        path = npmrc_file(args)
        if args.action == "npmrc":
            token = sys.stdin.read().strip()
            if not token or re.fullmatch(r"[A-Za-z0-9_]+", token) is None:
                raise ValueError("gh returned an empty or invalid token")
            lines = path.read_text().splitlines() if path.exists() else []
            lines = [line for line in lines if not line.strip().startswith(("@risingtides-dev:registry=", TOKEN_KEY))]
            atomic_write(path, "\n".join([*lines, SCOPE, TOKEN_KEY + token, ""]))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        # Do not print exception details: filesystem/parse errors can contain source bytes.
        print("Onboarding configuration refused; check file ownership, format, paths and inputs.", file=sys.stderr)
        sys.exit(1)
