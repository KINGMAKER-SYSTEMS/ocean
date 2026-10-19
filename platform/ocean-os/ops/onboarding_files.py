#!/usr/bin/env python3
"""Owner-only onboarding files; Python 3.11+ for the daemon's strict TOML shape."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import tomllib
import unicodedata

SCOPE = "@risingtides-dev:registry=https://npm.pkg.github.com"
TOKEN_KEY = "//npm.pkg.github.com/:_authToken="
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


def configuration_path(raw):
    path = Path(raw)
    if not path.is_absolute() or ".." in path.parts:
        raise ValueError("configuration paths must be absolute without parent traversal")
    return path


def owned_directory(fd):
    meta = os.fstat(fd)
    if not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.getuid():
        raise ValueError("configuration directory must be owned by this user")


@contextmanager
def parent_directory(path, create=False):
    # Walk from the filesystem root using descriptors. No checked pathname is
    # later reopened, so a concurrent rename/symlink cannot redirect custody.
    fd = os.open("/", DIRECTORY_FLAGS)
    try:
        for component in path.parent.parts[1:]:
            try:
                child = os.open(component, DIRECTORY_FLAGS, dir_fd=fd)
            except FileNotFoundError:
                if not create:
                    yield None
                    return
                try:
                    os.mkdir(component, mode=0o700, dir_fd=fd)
                except FileExistsError:
                    pass
                child = os.open(component, DIRECTORY_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = child
        owned_directory(fd)
        yield fd
    finally:
        os.close(fd)


@contextmanager
def owned_file(directory, name):
    # NONBLOCK avoids waiting on a FIFO before fstat can reject it.
    try:
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=directory)
    except FileNotFoundError:
        yield None
        return
    try:
        meta = os.fstat(fd)
        if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.getuid() or meta.st_nlink != 1:
            raise ValueError("configuration destination must be an owned regular file with one link")
        yield fd
    finally:
        os.close(fd)


def read_file(fd):
    with os.fdopen(os.dup(fd), encoding="utf-8") as source:
        return source.read()


def valid_member(value):
    return isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9._@-]+", value) is not None


def identity_file(args, write=False):
    if not valid_member(args.member):
        raise ValueError("member must be a nonempty ASCII identifier: letters, digits, . _ @ -")
    if len(args.display_name) > 80 or any(unicodedata.category(c) == "Cc" for c in args.display_name):
        raise ValueError("display name must be at most 80 characters without controls")
    path = configuration_path(args.config_dir) / "member.toml"
    with parent_directory(path, create=write) as directory:
        if directory is None:
            return path
        with owned_file(directory, path.name) as fd:
            if fd is not None and not args.force:
                try:
                    current = tomllib.loads(read_file(fd))
                    valid = set(current) <= {"member_id", "display_name"} and valid_member(current.get("member_id"))
                    valid = valid and ("display_name" not in current or isinstance(current["display_name"], str))
                except (ValueError, UnicodeError):
                    valid = False
                if not valid:
                    raise ValueError("existing member.toml is malformed; inspect it or use --force")
                if current["member_id"] != args.member:
                    raise ValueError("existing member.toml names another member; use --force only for an intentional replacement")
        if write:
            text = f"member_id = {json.dumps(args.member, ensure_ascii=False)}\n"
            if args.display_name:
                # JSON basic-string escapes are valid TOML for these control-free values.
                text += f"display_name = {json.dumps(args.display_name, ensure_ascii=False)}\n"
            atomic_write(directory, path.name, text, private_directory=True)
    return path


def npmrc_file(args):
    path = configuration_path(args.npmrc)
    with parent_directory(path) as directory:
        if directory is not None:
            with owned_file(directory, path.name):
                pass  # Validation, including dry-run, does not read credentials.
    return path


def atomic_write(directory, destination, text, private_directory=False):
    with owned_file(directory, destination):
        pass
    if private_directory:
        os.fchmod(directory, 0o700)
    name = f".{destination}.{secrets.token_hex(16)}"
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(text)
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, destination, src_dir_fd=directory, dst_dir_fd=directory)
        os.fsync(directory)
    finally:
        try:
            os.unlink(name, dir_fd=directory)
        except FileNotFoundError:
            pass


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
        identity_file(args, write=args.action == "identity")
    if args.action in {"validate", "npmrc"}:
        path = npmrc_file(args)
        if args.action == "npmrc":
            token = sys.stdin.read().strip()
            if not token or re.fullmatch(r"[A-Za-z0-9_]+", token) is None:
                raise ValueError("gh returned an empty or invalid token")
            with parent_directory(path, create=True) as directory:
                with owned_file(directory, path.name) as fd:
                    lines = read_file(fd).splitlines() if fd is not None else []
                lines = [line for line in lines if not line.strip().startswith(("@risingtides-dev:registry=", TOKEN_KEY))]
                atomic_write(directory, path.name, "\n".join([*lines, SCOPE, TOKEN_KEY + token, ""]))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError):
        # Do not print exception details: filesystem/parse errors can contain source bytes.
        print("Onboarding configuration refused; check file ownership, format, paths and inputs.", file=sys.stderr)
        sys.exit(1)
