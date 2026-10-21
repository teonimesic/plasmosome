#!/usr/bin/env python3
"""Regressions for the base check in the root README's status-only quickstart.

Each case runs the README's own block against a base made here. A stub stands in for the build,
so a base that passes the check stops there and nothing compiles. Run it as a user other than
root, because root can write a 0555 directory. Set QUICKSTART_NOEXEC_BASE to a private base on a
noexec mount to add the case that uses a real mount.
"""

import errno
import os
from pathlib import Path
import re
import secrets
import shutil
import stat
import string
import subprocess
import sys
import tempfile


REPOSITORY = Path(__file__).resolve().parent.parent
BUILD_REACHED = 3
LONGEST_BASE = 100 - 1 - len("/plasmosome-status-XXXXXXXX/membrane.uds")
PRELUDE = """
import os, shutil, subprocess, sys
shutil.which = lambda name: "/stub/" + name
def build(*arguments, **options):
    raise SystemExit({reached})
subprocess.Popen = build
{patch}
exec(compile(sys.stdin.read(), "README quickstart", "exec"), {{"__name__": "__main__"}})
"""
EFFECTIVE_ONLY = """
os.getuid = lambda: os.geteuid() + 1
real_access = os.access
def access(path, mode, *, effective_ids=False, **options):
    return effective_ids and real_access(path, mode, effective_ids=True, **options)
os.access = access
"""
STRANGER = 4242
ROOT = None
MADE = []


class Skipped(Exception):
    pass


def noexec_at(base):
    return f"""
import types
real_statvfs = os.statvfs
os.ST_NOEXEC = getattr(os, "ST_NOEXEC", 8)
def statvfs(path):
    if os.path.realpath(path) == {str(base)!r}:
        return types.SimpleNamespace(f_flag=os.ST_NOEXEC)
    return real_statvfs(path)
os.statvfs = statvfs
"""


def quickstart():
    text = (REPOSITORY / "README.md").read_text(encoding="utf-8")
    blocks = re.findall(r"^python3 - <<'PY'\n(.*?)^PY\n", text, re.M | re.S)
    if len(blocks) != 1:
        sys.exit(f"README.md holds {len(blocks)} quickstart blocks, not one")
    return blocks[0]


def private(path):
    try:
        for directory in [*reversed(path.parents), path]:
            info = os.lstat(directory)
            if not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, os.geteuid()):
                return False
            if info.st_mode & 0o022:
                return False
    except OSError:
        return False
    return True


def choose_root():
    candidates = [tempfile.gettempdir(), os.environ.get("XDG_RUNTIME_DIR"), os.environ.get("HOME")]
    for candidate in filter(None, candidates):
        path = Path(os.path.realpath(candidate))
        if private(path) and len(os.fsencode(path)) <= LONGEST_BASE - 3:
            return path
    sys.exit(f"none of {candidates} is a private directory under {LONGEST_BASE - 2} bytes")


def base_of(length, mode=0o700, directory=True, ending=""):
    size = length - len(os.fsencode(ROOT)) - 1 - len(ending.encode("utf-8"))
    while True:
        letters = "".join(secrets.choice(string.ascii_lowercase) for _ in range(size))
        path = ROOT / (letters + ending)
        try:
            path.mkdir() if directory else path.touch(exist_ok=False)
        except FileExistsError:
            continue
        MADE.append(path)
        os.chmod(path, mode)
        return path


def run(base, patch=""):
    return subprocess.run(
        [sys.executable, "-c", PRELUDE.format(reached=BUILD_REACHED, patch=patch)],
        input=SOURCE,
        cwd=REPOSITORY,
        env={**os.environ, "PLASMOSOME_QUICKSTART_DIR": str(base)},
        capture_output=True,
        encoding="utf-8",
        errors="backslashreplace",
        timeout=60,
    )


def described(result):
    return f"exit {result.returncode}\nstdout:\n{result.stdout}stderr:\n{result.stderr}"


def refused(base, reason, patch=""):
    result = run(base, patch)
    lines = result.stderr.splitlines()
    assert result.returncode == 1, described(result)
    assert len(lines) == 1 and lines[0].startswith(f"quickstart: {reason}"), described(result)
    assert "private base:" not in result.stdout, described(result)
    assert not base.is_dir() or not os.listdir(base), f"{base} was changed"


def passes(base, patch=""):
    result = run(base, patch)
    assert result.returncode == BUILD_REACHED, described(result)
    assert f"private base: {base}\n" in result.stdout, described(result)


def a_private_base_of_the_longest_length_reaches_the_build():
    passes(base_of(LONGEST_BASE))


def a_base_one_byte_too_long_is_refused_in_one_line():
    base = base_of(LONGEST_BASE + 1)
    refused(base, f"{base} is too long")


def a_base_one_byte_too_long_through_a_two_byte_character_is_refused():
    base = base_of(LONGEST_BASE + 1, ending="\u00e9")
    refused(base, f"{base} is too long")


def a_base_that_is_not_valid_utf8_is_refused():
    base = ROOT / os.fsdecode(b"q\xff" + secrets.token_hex(4).encode())
    try:
        base.mkdir(mode=0o700)
        MADE.append(base)
    except OSError as error:
        if error.errno != errno.EILSEQ:
            raise
    shown = os.fsencode(base).decode("utf-8", "backslashreplace")
    refused(base, f"{shown} is not valid UTF-8")


def a_base_whose_owner_is_not_the_effective_uid_is_refused():
    base = base_of(LONGEST_BASE)
    chain = [*reversed(base.parents), base]
    first = next(d for d in chain if os.lstat(d).st_uid not in (0, STRANGER))
    owner = os.lstat(first).st_uid
    reason = f"{first} is owned by uid {owner}, not by you or root"
    refused(base, reason, f"os.geteuid = lambda: {STRANGER}")


def a_missing_base_is_refused():
    base = ROOT / f"missing-{secrets.token_hex(4)}"
    refused(base, f"cannot inspect {base}: No such file or directory")


def a_file_is_refused():
    base = base_of(LONGEST_BASE, 0o600, directory=False)
    refused(base, f"{base} is not a directory")


def a_base_group_can_write_is_refused():
    base = base_of(LONGEST_BASE, 0o770)
    refused(base, f"{base} has mode 0770: group or other can replace entries in it")


def a_base_without_write_permission_is_refused():
    base = base_of(LONGEST_BASE, 0o555)
    refused(base, f"you cannot create entries in {base}")


def a_base_without_search_permission_is_refused():
    base = base_of(LONGEST_BASE, 0o600)
    refused(base, f"you cannot create entries in {base}")


def a_base_on_a_noexec_mount_is_refused():
    base = base_of(LONGEST_BASE)
    refused(base, f"{base} is on a noexec mount", noexec_at(base))


def a_base_on_a_real_noexec_mount_is_refused():
    real = os.environ.get("QUICKSTART_NOEXEC_BASE")
    if not real:
        raise Skipped("needs a prepared noexec mount named by QUICKSTART_NOEXEC_BASE, unset here")
    base = Path(os.path.realpath(real))
    refused(base, f"{base} is on a noexec mount")


def a_base_only_the_effective_uid_can_write_reaches_the_build():
    passes(base_of(LONGEST_BASE), EFFECTIVE_ONLY)


TESTS = [
    a_private_base_of_the_longest_length_reaches_the_build,
    a_base_one_byte_too_long_is_refused_in_one_line,
    a_base_one_byte_too_long_through_a_two_byte_character_is_refused,
    a_base_that_is_not_valid_utf8_is_refused,
    a_base_whose_owner_is_not_the_effective_uid_is_refused,
    a_missing_base_is_refused,
    a_file_is_refused,
    a_base_group_can_write_is_refused,
    a_base_without_write_permission_is_refused,
    a_base_without_search_permission_is_refused,
    a_base_on_a_noexec_mount_is_refused,
    a_base_on_a_real_noexec_mount_is_refused,
    a_base_only_the_effective_uid_can_write_reaches_the_build,
]


def main():
    global ROOT
    if os.geteuid() == 0:
        sys.exit("run as a user other than root: root can write a 0555 directory")
    ROOT = choose_root()
    failed, skipped = [], []
    try:
        for test in TESTS:
            try:
                test()
                print(f"ok {test.__name__}")
            except Skipped as reason:
                skipped.append(test.__name__)
                print(f"skip {test.__name__}: {reason}")
            except AssertionError as error:
                failed.append(test.__name__)
                print(f"FAIL {test.__name__}: {error}")
    finally:
        for path in MADE:
            if path.is_dir():
                os.chmod(path, 0o700)
                shutil.rmtree(path)
            else:
                path.unlink()
    passed = len(TESTS) - len(failed) - len(skipped)
    print(f"{passed} passed, {len(failed)} failed, {len(skipped)} skipped, under {ROOT}")
    return 1 if failed else 0


SOURCE = quickstart()

if __name__ == "__main__":
    sys.exit(main())
