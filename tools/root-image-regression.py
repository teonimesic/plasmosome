#!/usr/bin/env python3
"""Defend tools/root-image: the image contract, its refusals, the pinned Docker sequence, placement.

Run all tests with `python3 tools/root-image-regression.py`, or name some to run only those.
"""

from collections import namedtuple
from contextlib import contextmanager, redirect_stderr, redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import posixpath
import re
import runpy
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import traceback


SOURCE = Path(__file__).resolve().with_name("root-image")
API = runpy.run_path(str(SOURCE))
TOOL = API["build"].__globals__
Refused, Result, Add = API["Refused"], API["Result"], API["Add"]

DIGEST = "7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587"
PLATFORM_DIGEST = "a1b86db52ce3daef089e45aabe36dfec4091f82464c25c1fdcf03de197cbe82a"
REF = f"debian:bookworm-slim@sha256:{DIGEST}"
CONTAINER = "4f1c0a9e2b7d"
NOW = 1_792_000_000
EPOCH = 1_700_000_000
BASE_MTIME = 1_759_622_400
LIMIT = 4 << 20
ENVIRON = {"PATH": "/usr/bin:/bin", "HOME": "/Users/someone", "DOCKER_CONTEXT": "desktop-linux",
           "GITHUB_TOKEN": "not for docker", "LANG": "C"}
ENV = {"PATH": "/usr/bin:/bin", "HOME": "/Users/someone", "DOCKER_CONTEXT": "desktop-linux"}
SIZE_MIB = 1
ROOTFS = b"ext4 image bytes".ljust(SIZE_MIB << 20, b"\0")

USERS = b"root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n"
GROUP = b"root:x:0:\nshadow:x:42:\nnogroup:x:65534:\n"
SHADOW = b"root:*:20731:0:99999:7:::\nnobody:*:20731:0:99999:7:::\n"
GSHADOW = b"root:*::\nshadow:*::\nnogroup:*::\n"
WORKLOAD_USER = b"workload:x:1000:1000::/home/workload:/bin/bash\n"
WORKLOAD_GROUP = b"workload:x:1000:\n"
WORKLOAD_SHADOW = b"workload:!:19675:0:99999:7:::\n"
WORKLOAD_GSHADOW = b"workload:!::\n"
BASHRC, PROFILE = b"# ~/.bashrc\n", b"# ~/.profile\n"
BANNER = "debugfs 1.47.0 (5-Feb-2023)\n"
MKE2FS_VERSION = "mke2fs 1.47.0 (5-Feb-2023)\n\tUsing EXT2FS Library version 1.47.0\n"
FEATURES = "has_journal ext_attr extent 64bit flex_bg metadata_csum"

Member = namedtuple("Member", "kind mode uid gid data link mtime")


def d(mode=0o755, uid=0, gid=0):
    return ("dir", mode, uid, gid, b"", "")


def f(data=b"", mode=0o644, uid=0, gid=0):
    return ("file", mode, uid, gid, data, "")


def s(target):
    return ("symlink", 0o777, 0, 0, b"", target)


def h(target):
    return ("hardlink", 0o755, 0, 0, b"", target)


DEBIAN = {
    ".dockerenv": f(mode=0o755),
    "bin": s("usr/bin"),
    "dev": d(), "dev/console": f(mode=0o755), "dev/pts": d(), "dev/shm": d(),
    "etc": d(),
    "etc/debian_version": f(b"12.15\n"),
    "etc/group": f(GROUP),
    "etc/gshadow": f(GSHADOW, 0o640, 0, 42),
    "etc/hostname": f(mode=0o755), "etc/hosts": f(mode=0o755), "etc/mtab": s("/proc/mounts"),
    "etc/passwd": f(USERS),
    "etc/resolv.conf": f(mode=0o755),
    "etc/shadow": f(SHADOW, 0o640, 0, 42),
    "etc/skel": d(), "etc/skel/.bashrc": f(BASHRC), "etc/skel/.profile": f(PROFILE),
    "home": d(), "opt": d(), "proc": d(), "sys": d(), "tmp": d(0o1777),
    "usr": d(), "usr/bin": d(), "usr/bin/bash": f(b"\x7fELF bash", 0o755),
    "usr/bin/gunzip": f(b"gunzip", 0o755), "usr/bin/uncompress": h("usr/bin/gunzip"),
    "usr/local": d(), "usr/local/bin": d(),
    "var": d(), "var/mail": d(0o2775, 0, 8),
}
TYPES = {"dir": tarfile.DIRTYPE, "file": tarfile.REGTYPE, "symlink": tarfile.SYMTYPE,
         "hardlink": tarfile.LNKTYPE, "fifo": tarfile.FIFOTYPE, "char": tarfile.CHRTYPE}


def sha(data):
    return hashlib.sha256(data).hexdigest()


@contextmanager
def scratch():
    root = Path(tempfile.mkdtemp(prefix="root-image-test-"))
    try:
        yield root
    finally:
        for path, _, _ in os.walk(root):
            os.chmod(path, 0o700)
        shutil.rmtree(root)


@contextmanager
def patched(name, value):
    original = TOOL[name]
    TOOL[name] = value
    try:
        yield
    finally:
        TOOL[name] = original


def write_tar(path, entries, mtimes=None):
    with tarfile.open(path, "w", format=tarfile.GNU_FORMAT) as out:
        for name, spec in entries.items():
            if spec is None:
                continue
            kind, mode, uid, gid, data, link = spec
            info = tarfile.TarInfo(name)
            info.type, info.mode, info.uid, info.gid = TYPES[kind], mode, uid, gid
            info.mtime, info.linkname, info.size = (mtimes or {}).get(name, BASE_MTIME), link, len(data)
            out.addfile(info, io.BytesIO(data) if kind == "file" else None)


def read_tar(path):
    tree = {}
    with tarfile.open(path) as source:
        for member in source:
            kind = ("dir" if member.isdir() else "symlink" if member.issym() else
                    "hardlink" if member.islnk() else "file" if member.isreg() else "other")
            data = source.extractfile(member).read() if kind == "file" else b""
            tree[member.name] = Member(kind, member.mode, member.uid, member.gid, data,
                                       member.linkname, member.mtime)
    return tree


def staged(root, changes=None, adds=(), epoch=EPOCH, limit=LIMIT):
    base, out = root / "base.tar", root / "stage.tar"
    write_tar(base, {**DEBIAN, **(changes or {})})
    facts = API["stage"](base, out, list(adds), epoch, limit)
    return read_tar(out), facts


def stage_with(root, changes=None, adds=(), limit=LIMIT):
    staged(root, changes, adds, limit=limit)


def refusal(step, call, *args, **kwargs):
    try:
        call(*args, **kwargs)
    except Refused as error:
        assert error.step == step, f"refused at {error.step!r}, wanted {step!r}: {error.detail}"
        return error.detail
    raise AssertionError(f"{call.__name__} did not refuse")


def debugfs_stat(member):
    kind = {"dir": "directory", "file": "regular", "symlink": "symlink"}[member.kind]
    return (f"Inode: 12   Type: {kind}    Mode:  0{member.mode:03o}   Flags: 0x80000\n"
            "Generation: 0    Version: 0x00000000:00000000\n"
            f"User:  {member.uid:4d}   Group:  {member.gid:4d}   Project:     0   Size: {len(member.data)}\n")


def debugfs_ls(tree, directory):
    bits = {"dir": 0o40000, "file": 0o100000, "hardlink": 0o100000, "symlink": 0o120000}
    here = tree[directory]
    rows = [f"/2/{bits['dir'] | here.mode:06o}/{here.uid}/{here.gid}/.//", "/2/040755/0/0/..//"]
    for name, member in tree.items():
        if posixpath.dirname(name) == directory:
            rows.append(f"/13/{bits[member.kind] | member.mode:06o}/{member.uid}/{member.gid}/"
                        f"{posixpath.basename(name)}/{len(member.data)}/")
    return "\n".join(rows) + "\n\n"


class FakeDocker:
    """A recording runner: Docker's answers, and a container that reads the stage like debugfs."""

    def __init__(self, base=None):
        self.base = {**DEBIAN, **(base or {})}
        self.calls, self.envs = [], []
        self.missing, self.e2fsck, self.run_code, self.rootfs = False, "0", 0, ROOTFS
        self.unshared, self.inspect_out, self.create_code, self.export_code, self.rm_code = False, None, 0, 0, 0
        self.repo_digests = [f"debian@sha256:{DIGEST}"]
        self.platform = {"Os": "linux", "Architecture": "arm64"}
        self.descriptor = {"mediaType": "application/vnd.oci.image.index.v1+json", "digest": f"sha256:{DIGEST}"}
        self.create_out, self.after, self.exported = CONTAINER + "\n", None, None
        self.platform_descriptor = {"mediaType": "application/vnd.oci.image.manifest.v1+json",
                                    "digest": f"sha256:{PLATFORM_DIGEST}"}

    def __call__(self, argv, env):
        self.calls.append(list(argv))
        self.envs.append(env)
        if argv[:3] == ["docker", "image", "inspect"]:
            if self.missing:
                return Result(1, "[]\n", f"Error response from daemon: No such image: {argv[-1]}\n")
            if "--platform" in argv:
                return Result(0, json.dumps([{**self.platform, "Descriptor": self.platform_descriptor}]), "")
            if self.inspect_out is not None:
                return Result(0, self.inspect_out, "")
            record = {"Id": f"sha256:{DIGEST}", "RepoDigests": self.repo_digests}
            return Result(0, json.dumps([{**record, **({"Descriptor": self.descriptor} if self.descriptor else {})}]), "")
        if argv[:2] == ["docker", "create"]:
            return Result(self.create_code, self.create_out if self.create_code == 0 else "", "no create\n")
        if argv[:2] == ["docker", "export"]:
            write_tar(Path(argv[3]), self.base)
            if self.exported is not None:
                self.exported(Path(argv[3]).parent)
            return Result(self.export_code, "", "no export\n")
        if argv[:2] == ["docker", "rm"]:
            return Result(self.rm_code, CONTAINER + "\n", "")
        if argv[:2] == ["docker", "run"]:
            mount = argv[argv.index("-v") + 1]
            assert mount.endswith(":/w"), mount
            if self.unshared:
                message = re.search(r"echo '([^']+)' >&2; exit 3", argv[-1]).group(1)
                return Result(3, "", message + "\n")
            if self.run_code == 0:
                self.container(Path(mount[:-3]), argv[-1])
            return Result(self.run_code, "", "docker: exited\n" if self.run_code else "")
        if argv[0] == "chflags":
            return Result(0, "", "")
        raise AssertionError(f"unexpected command {argv}")

    def container(self, work, script):
        tree = read_tar(work / "stage.tar")
        uuid = re.search(r" -U (\S+) ", script).group(1)
        check = work / "check"
        if self.rootfs is not None:
            (work / "rootfs.raw").write_bytes(self.rootfs)
        (check / "mke2fs-version").write_text(MKE2FS_VERSION)
        (check / "e2fsck.code").write_text(f"{self.e2fsck}\n")
        (check / "e2fsck.out").write_text("e2fsck 1.47.0 (5-Feb-2023)\nInode 12 has illegal block(s).\n")
        (check / "dumpe2fs").write_text(f"Filesystem volume name:   <none>\nFilesystem UUID:          {uuid}\n"
                                        f"Filesystem features:      {FEATURES}\nBlock size:               4096\n")
        for verb, path, name in re.findall(r"debugfs -R '(cat|stat|ls -p) (\S+)' /w/rootfs.raw "
                                           r">/w/check/(\S+) 2>", script):
            key = path.lstrip("/")
            if key not in tree:
                (check / name).write_bytes(b"")
                (check / f"{name}.err").write_text(f"{BANNER}{path}: File not found by ext2_lookup \n")
                continue
            output = {"cat": lambda: tree[key].data, "stat": lambda: debugfs_stat(tree[key]).encode(),
                      "ls -p": lambda: debugfs_ls(tree, key).encode()}[verb]()
            (check / name).write_bytes(output)
            (check / f"{name}.err").write_text(BANNER)
        if self.after is not None:
            self.after(work)


def built(root, fake, base=REF, adds=(), extra=(), work_name="work"):
    work, images = root / work_name, root / "images"
    images.mkdir(exist_ok=True)
    argv = ["build", "--base", base, "--work-root", str(work), "--images-root", str(images),
            "--source-date-epoch", str(EPOCH), "--size-mib", str(SIZE_MIB), *extra]
    for add in adds:
        argv += ["--add", add]
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = API["main"](argv, run=fake, now=lambda: NOW, environ=ENVIRON)
    return code, out.getvalue(), err.getvalue(), work, images


def stage_adds_the_workload_account_home_and_tmp():
    with scratch() as root:
        tree, facts = staged(root, {"tmp": d(0o755, 501, 80), "etc/group": f(GROUP.rstrip(b"\n"))})
        assert tree["etc/passwd"][:5] == ("file", 0o644, 0, 0, USERS + WORKLOAD_USER)
        assert tree["etc/group"][:5] == ("file", 0o644, 0, 0, GROUP + WORKLOAD_GROUP)
        assert tree["etc/shadow"][:5] == ("file", 0o640, 0, 42, SHADOW + WORKLOAD_SHADOW)
        assert tree["etc/gshadow"][:5] == ("file", 0o640, 0, 42, GSHADOW + WORKLOAD_GSHADOW)
        assert tree["home/workload"][:4] == ("dir", 0o700, 1000, 1000)
        assert tree["home/workload/.bashrc"][:5] == ("file", 0o644, 1000, 1000, BASHRC)
        assert tree["home/workload/.profile"][:5] == ("file", 0o644, 1000, 1000, PROFILE)
        assert tree["tmp"][:4] == ("dir", 0o1777, 0, 0)
        assert {tree[name].mtime for name in ("etc/passwd", "home/workload", "home/workload/.bashrc", "tmp")} == {EPOCH}
        assert tree["usr/bin/bash"].mtime == BASE_MTIME and tree["var/mail"][:4] == ("dir", 0o2775, 0, 8)
        assert facts["skel"] == [".bashrc", ".profile"] and facts["debian_version"] == "12.15"
        assert "/tmp/leftover" in refusal("stage", stage_with, root, {"tmp/leftover": f(b"x")})
        assert "/bin/bash" in refusal("stage", stage_with, root, {"usr/bin/bash": None})
        assert "/bin/bash" in refusal("stage", stage_with, root, {"bin": s("bin")})
        assert "/bin/bash" in refusal("stage", stage_with, root, {"usr/bin/bash": f(b"bash", 0o644)})
        tree, _ = staged(root, {"bin": s("./usr/../usr//libexec"), "usr/libexec": d(),
                                "usr/libexec/bash": h("usr/bin/gunzip")})
        assert tree["bin"].link == "./usr/../usr//libexec" and tree["usr/libexec/bash"].kind == "hardlink"
        assert "/etc/skel/.config" in refusal("stage", stage_with, root, {"etc/skel/.config": d()})
        assert "/home/workload" in refusal("stage", stage_with, root, {"home/workload": d()})
        assert "/etc/shadow" in refusal("stage", stage_with, root, {"etc/shadow": None})


def an_existing_workload_name_or_id_1000_refuses():
    with scratch() as root:
        for path, data in [
            ("etc/passwd", USERS + b"workload:x:1001:1001::/home/w:/bin/sh\n"),
            ("etc/passwd", USERS + b"alice:x:1000:1000::/home/alice:/bin/sh\n"),
            ("etc/group", GROUP + b"staff:x:1000:\n"),
            ("etc/group", GROUP + b"workload:x:1001:\n"),
            ("etc/shadow", SHADOW + b"workload:*:20731:0:99999:7:::\n"),
            ("etc/gshadow", GSHADOW + b"workload:*::\n"),
            ("etc/passwd", USERS + b"alice:x:1001:1000::/home/alice:/bin/sh\n"),
            ("etc/group", GROUP + b"sudo:x:27:workload\n"),
            ("etc/group", GROUP + b"staff:x:50:alice,workload\n"),
            ("etc/gshadow", GSHADOW + b"sudo:*::workload\n"),
            ("etc/gshadow", GSHADOW + b"staff:*:workload:\n"),
            ("etc/passwd", USERS + b" workload:x:1001:1001::/home/w:/bin/sh\n"),
            ("etc/passwd", USERS + b"\tworkload:x:1001:1001::/home/w:/bin/sh\n"),
            ("etc/group", GROUP + b" workload:x:1001:\n"),
            ("etc/group", GROUP + b"sudo:x:27: workload\n"),
            ("etc/group", GROUP + b"staff:x:50:alice, workload\n"),
            ("etc/shadow", SHADOW + b" workload:$6$salt$hash:20000:0:99999:7:::\n"),
            ("etc/gshadow", GSHADOW + b" workload:*::\n"),
            ("etc/gshadow", GSHADOW + b"sudo:*: workload:\n"),
            ("etc/gshadow", GSHADOW + b"sudo:*:: workload\n"),
            ("etc/gshadow", GSHADOW + b"\r\x0b\x0cworkload:*::\n"),
        ]:
            _, mode, uid, gid, _, _ = DEBIAN[path]
            detail = refusal("stage", stage_with, root, {path: f(data, mode, uid, gid)})
            assert f"/{path}" in detail and "already" in detail, detail
        for path, data in [
            ("etc/passwd", USERS + b"alice:x:1001: 1000::/home/alice:/bin/sh\n"),
            ("etc/passwd", USERS + b"alice:x:1001:+1000::/home/alice:/bin/sh\n"),
            ("etc/passwd", USERS + b"alice:x:-4294966296:1001::/home/alice:/bin/sh\n"),
            ("etc/group", GROUP + b"staff:x:01000:\n"),
            ("etc/passwd", USERS + b"alice:x:1001:" + b"1" * 5000 + b"::/home/alice:/bin/sh\n"),
        ]:
            detail = refusal("stage", stage_with, root, {path: f(data)})
            assert f"/{path} line" in detail, detail


def entries_under_opt_or_usr_local_bin_refuse():
    with scratch() as root:
        for changes, path in [({"opt/yarn-v1.22.22": d()}, "/opt/yarn-v1.22.22"),
                              ({"usr/local/bin/node": f(b"node", 0o755)}, "/usr/local/bin/node"),
                              ({"opt": s("var/opt")}, "/opt"), ({"usr/local/bin": s("../../bin")}, "/usr/local/bin")]:
            assert path in refusal("stage", stage_with, root, changes)


def the_product_image_has_nothing_under_opt_or_usr_local_bin():
    with scratch() as root:
        code, out, err, work, images = built(root, FakeDocker())
        assert code == 0, err
        image = Path(json.loads(out)["image_dir"])
        records = [json.loads(line) for line in (image / "content.jsonl").read_text().splitlines()]
        paths = {record["path"]: record for record in records}
        assert paths["/opt"]["type"] == "dir" and paths["/usr/local/bin"]["type"] == "dir"
        assert [p for p in paths if p.startswith(("/opt/", "/usr/local/bin/"))] == []
        code, out, err, work, images = built(root, FakeDocker({"opt": None, "usr/local/bin": None}), work_name="absent")
        assert code == 0, err
        records = (Path(json.loads(out)["image_dir"]) / "content.jsonl").read_text()
        assert '"/opt"' not in records and '"/usr/local/bin"' not in records


def dev_proc_sys_are_emptied_and_dockerenv_dropped():
    with scratch() as root:
        tree, _ = staged(root, {"proc/1": d(), "sys": None, "sys/kernel": d(), "dev": d(0o700, 501, 80),
                                "proc": d(0o555, 1, 1), "dev/null": ("char", 0o666, 0, 0, b"", "")})
        for name in ("dev", "proc", "sys"):
            assert tree[name][:4] == ("dir", 0o755, 0, 0), (name, tree[name])
        assert [name for name in tree if name.startswith(("dev/", "proc/", "sys/"))] == []
        assert ".dockerenv" not in tree


def a_malformed_base_member_refuses():
    with scratch() as root:
        fifo = ("fifo", 0o644, 0, 0, b"", "")
        assert "/run/initctl" in refusal("stage", stage_with, root, {"run": d(), "run/initctl": fifo})
        assert "../escape" in refusal("stage", stage_with, root, {"../escape": f()})
        for name in ("./etc/hostname", "home//workload/x"):
            assert "normalised" in refusal("stage", stage_with, root, {name: f()}), name
        fake = FakeDocker({"etc/passwd": f(USERS + b"+alice:x:1001:1001::/home/alice:/bin/sh\n")})
        code, out, err, work, images = built(root, fake, work_name="nis")
        assert code == 1 and err.startswith("root-image: refused stage: /etc/passwd line 3 is malformed"), err
        assert ["docker", "run"] not in [argv[:2] for argv in fake.calls], fake.calls
        base = root / "base.tar"
        write_tar(base, DEBIAN)
        with tarfile.open(base, "a") as out:
            info = tarfile.TarInfo("etc/passwd")
            info.size = 1
            out.addfile(info, io.BytesIO(b"x"))
        assert "/etc/passwd" in refusal("stage", API["stage"], base, root / "out.tar", [], EPOCH, LIMIT)
        assert "/usr/bin/zcat" in refusal("stage", stage_with, root, {"usr/bin/zcat": h("usr/bin/absent")})
        assert "/etc/passwd2" in refusal("stage", stage_with, root, {"etc/passwd2": h("etc/passwd")})
        for target in ("bin", "usr"):
            detail = refusal("stage", stage_with, root, {"usr/bin/zz": h(target)})
            assert "/usr/bin/zz" in detail and "not an earlier file" in detail, detail
        base.write_bytes(b"not a tar archive" * 64)
        assert "tar" in refusal("stage", API["stage"], base, root / "out.tar", [], EPOCH, LIMIT)


def network_files_are_fixed():
    with scratch() as root:
        tree, _ = staged(root, {"etc/hostname": f(b"4f1c0a9e2b7d\n", 0o755),
                                "etc/resolv.conf": s("../run/systemd/resolve/stub-resolv.conf"),
                                "etc/hosts": None})
        assert tree["etc/hostname"][:5] == ("file", 0o644, 0, 0, b"localhost\n")
        assert tree["etc/hosts"][:5] == (
            "file", 0o644, 0, 0, b"127.0.0.1 localhost\n::1 localhost ip6-localhost ip6-loopback\n")
        assert tree["etc/resolv.conf"][:5] == ("file", 0o644, 0, 0, b"nameserver 127.0.0.53\n")
        assert tree["etc/hostname"].mtime == EPOCH
        assert "/etc/hostname" in refusal("stage", stage_with, root, {"etc/hostname": d()})


def a_member_naming_plasmosome_refuses():
    with scratch() as root:
        for changes, name in [({"usr/share": d(), "usr/share/Plasmosome": f()}, "/usr/share/Plasmosome"),
                              ({"etc/PLASMID.conf": f()}, "/etc/PLASMID.conf"),
                              ({"etc/hint": s("/run/plasmid")}, "/etc/hint"),
                              ({"usr/bin/pla\u017fmid": f()}, "/usr/bin/pla\u017fmid")]:
            assert name in refusal("stage", stage_with, root, changes)
        probe = root / "probe"
        probe.write_bytes(b"probe")
        detail = refusal("stage", stage_with, root, adds=[Add(str(probe), "/usr/bin/plasmid-probe", 0o755)])
        assert "/usr/bin/plasmid-probe" in detail


def add_places_a_root_owned_file_and_refuses_bad_destinations():
    with scratch() as root:
        probe, marker = root / "probe", root / "marker"
        probe.write_bytes(b"\x7fELF probe")
        marker.write_bytes(b"marker\n")
        adds = [API["parse_add"](f"{probe}:/usr/bin/lane-probe:0755"), API["parse_add"](f"{marker}:/image-marker:644")]
        assert adds == [Add(str(probe), "/usr/bin/lane-probe", 0o755), Add(str(marker), "/image-marker", 0o644)]
        tree, facts = staged(root, adds=adds)
        assert tree["usr/bin/lane-probe"] == Member("file", 0o755, 0, 0, b"\x7fELF probe", "", EPOCH)
        assert tree["image-marker"] == Member("file", 0o644, 0, 0, b"marker\n", "", EPOCH)
        assert facts["added"] == [{"dest": "/usr/bin/lane-probe", "mode": "0755", "sha256": sha(b"\x7fELF probe")},
                                  {"dest": "/image-marker", "mode": "0644", "sha256": sha(b"marker\n")}]
        for dest in ["usr/bin/x", "/usr/../etc/x", "/usr//bin/x", "/usr/bin/./x", "/usr/bin/", "/", "/opt",
                     "/opt/x", "/usr/local/bin", "/usr/local/bin/x", "/proc/x", "/dev/x", "/sys/x", "/tmp/x",
                     "/home/workload", "/home/workload/.bashrc", "/usr/bin/x y", "/usr/bin/x'"]:
            refusal("add", API["parse_add"], f"{probe}:{dest}:0755")
        for mode in ["4755", "2755", "1755", "0o755", "755x", "10644", "75", ""]:
            refusal("add", API["parse_add"], f"{probe}:/usr/bin/x:{mode}")
        refusal("add", API["parse_add"], "/usr/bin/x:0755")
        for dest in ["/etc/passwd", "/usr/bin/bash", "/usr/bin/uncompress", "/bin/x", "/nope/x", "/etc/skel/.bashrc/x"]:
            assert dest in refusal("stage", stage_with, root, adds=[Add(str(probe), dest, 0o755)])
        twice = [Add(str(probe), "/usr/bin/x", 0o755)] * 2
        assert "/usr/bin/x" in refusal("stage", stage_with, root, adds=twice)
        assert "absent" in refusal("stage", stage_with, root, adds=[Add(str(root / "absent"), "/usr/bin/x", 0o755)])
        (root / "linked").symlink_to(probe)
        tree, _ = staged(root, adds=[Add(str(root / "linked"), "/usr/bin/x", 0o755)])
        assert tree["usr/bin/x"][:5] == ("file", 0o755, 0, 0, b"\x7fELF probe")
        marker.write_bytes(b"abc")
        with patched("MAX_ADD_BYTES", 2):
            detail = refusal("stage", stage_with, root, adds=[Add(str(marker), "/usr/bin/x", 0o755)])
        assert "/usr/bin/x" in detail and "larger than 2 bytes" in detail, detail


def the_kind_is_test_exactly_when_add_is_given():
    with scratch() as root:
        probe = root / "probe"
        probe.write_bytes(b"probe")
        for adds, kind in [((), "product"), ((f"{probe}:/usr/bin/lane-probe:0755",), "test")]:
            code, out, err, work, images = built(root, FakeDocker(), adds=adds, work_name=f"work-{kind}")
            assert code == 0, err
            result = json.loads(out)
            manifest = json.loads((Path(result["image_dir"]) / "manifest.json").read_text())
            assert result["kind"] == kind and manifest["kind"] == kind
            assert len(manifest["added"]) == len(adds)


def content_listing_is_sorted_and_stable():
    with scratch() as root:
        base = root / "base.tar"
        write_tar(base, DEBIAN)
        API["stage"](base, root / "a.tar", [], EPOCH, LIMIT)
        API["stage"](base, root / "b.tar", [], EPOCH, LIMIT)
        assert (root / "a.tar").read_bytes() == (root / "b.tar").read_bytes()
        listing = API["content_listing"](root / "a.tar")
        assert listing == API["content_listing"](root / "b.tar") and listing.endswith(b"\n")
        lines = listing.decode().splitlines()
        records = [json.loads(line) for line in lines]
        assert [r["path"] for r in records] == sorted(r["path"] for r in records)
        assert lines == [json.dumps(r, sort_keys=True, separators=(",", ":")) for r in records]
        paths = {r["path"]: r for r in records}
        users = USERS + WORKLOAD_USER
        assert paths["/etc/passwd"] == {"path": "/etc/passwd", "type": "file", "mode": "0644", "uid": 0, "gid": 0,
                                        "size": len(users), "sha256": sha(users), "link": None}
        assert paths["/bin"] == {"path": "/bin", "type": "symlink", "mode": "0777", "uid": 0, "gid": 0,
                                 "size": 0, "sha256": None, "link": "usr/bin"}
        assert paths["/usr/bin/uncompress"]["type"] == "hardlink"
        assert paths["/usr/bin/uncompress"]["link"] == "/usr/bin/gunzip"
        assert paths["/home/workload"]["mode"] == "0700" and paths["/tmp"]["mode"] == "1777"
        assert sha(listing) == sha(API["content_listing"](root / "a.tar"))
        write_tar(base, {**DEBIAN, "usr/bin/gunzip": f(b"gunzip", 0o700)})
        API["stage"](base, root / "c.tar", [], EPOCH, LIMIT)
        assert sha(API["content_listing"](root / "c.tar")) != sha(listing)


def docker_create_times_do_not_reach_the_stage():
    with scratch() as root:
        touched = (".dockerenv", "dev", "dev/console", "dev/pts", "dev/shm", "etc", "etc/hostname", "etc/hosts",
                   "etc/mtab", "etc/resolv.conf", "proc", "sys")
        for number, created in enumerate((1_792_521_222, 1_792_521_289)):
            write_tar(root / f"base-{number}.tar", DEBIAN, {name: created for name in touched})
            API["stage"](root / f"base-{number}.tar", root / f"stage-{number}.tar", [], EPOCH, LIMIT)
        assert (root / "stage-0.tar").read_bytes() == (root / "stage-1.tar").read_bytes()
        tree = read_tar(root / "stage-0.tar")
        assert {tree[name].mtime for name in ("dev", "etc", "etc/mtab", "proc", "sys")} == {EPOCH}
        assert tree["etc/mtab"][:6] == ("symlink", 0o777, 0, 0, b"", "/proc/mounts")
        assert tree["usr/bin/bash"].mtime == BASE_MTIME and tree["etc/skel"].mtime == BASE_MTIME


@contextmanager
def oversize(path, size):
    original = path.read_bytes()
    with open(path, "r+b") as out:
        out.truncate(size)
    try:
        yield
    finally:
        path.write_bytes(original)


@contextmanager
def changed(path, data):
    original = path.read_bytes() if path.exists() else None
    path.write_bytes(data.encode() if isinstance(data, str) else data)
    try:
        yield
    finally:
        if original is None:
            path.unlink()
        else:
            path.write_bytes(original)


def contract_check_reads_debugfs_output():
    with scratch() as root:
        probe = root / "probe"
        probe.write_bytes(b"probe")
        adds = [Add(str(probe), "/usr/bin/lane-probe", 0o755)]
        work = root / "work"
        (work / "check").mkdir(parents=True)
        base = root / "base.tar"
        write_tar(base, DEBIAN)
        facts = API["stage"](base, work / "stage.tar", adds, EPOCH, LIMIT)
        uuid = "01234567-89ab-cdef-0123-456789abcdef"
        FakeDocker().container(work, API["build_script"](uuid, 1024, adds))
        check = work / "check"
        assert API["check_image"](check, facts, uuid) == {"mke2fs_version": "1.47.0", "features": FEATURES.split()}
        home = (check / "home.stat").read_text()
        tmp = (check / "tmp.stat").read_text()
        users = (check / "passwd").read_bytes()
        cases = [
            ("home.stat", home.replace("Mode:  0700", "Mode:  0755"), "0755"),
            ("home.stat", home.replace("User:  1000", "User:     0"), "/home/workload"),
            ("home.stat", home.replace("Type: directory", "Type: regular"), "/home/workload"),
            ("home.stat", "", "/home/workload"),
            ("tmp.stat", tmp.replace("Mode:  01777", "Mode:  0777"), "/tmp"),
            ("tmp.stat", tmp.replace("User:     0", "User:   501"), "/tmp"),
            ("passwd", users + b"alice:x:1000:1000::/home/alice:/bin/sh\n", "1000"),
            ("passwd", users.replace(b":/home/workload:", b":/home/w:"), "workload"),
            ("passwd", users.replace(b"workload:x:1000:1000", b"workload:x:01000:1000"), "line 3"),
            ("passwd", users + b"+::::::\n", "line 4"),
            ("passwd", users.replace(b"\nnobody", b"\n\nnobody"), "line 2"),
            ("passwd", b"root:x:0:0\n", "line 1"),
            ("passwd", users + b"x" * (1 << 20), "/etc/passwd"),
            ("group", GROUP, "1000"),
            ("group", GROUP + b"workload:x:1000:alice\n", "workload"),
            ("group", GROUP + b"workload:x:4294967296:\n", "line 4"),
            ("opt.ls", "/2/040755/0/0/.//\n/2/040755/0/0/..//\n/14/0100755/0/0/yarn/10/\n\n", "/opt"),
            ("usr-local-bin.ls", "/2/040755/0/0/.//\n/2/040755/0/0/..//\n/14/0100755/0/0/node/9/\n\n", "node"),
            ("opt.ls", "", "/opt"),
            ("home.ls", (check / "home.ls").read_text().replace(".profile", ".prof1le"), ".profile"),
            ("home.ls", (check / "home.ls").read_text().replace("/1000/1000/.bashrc", "/0/0/.bashrc"), ".bashrc"),
            ("home.ls", (check / "home.ls").read_text().replace("/1000/1000/.bashrc", "/1000/0/.bashrc"), ".bashrc"),
            ("home.ls", (check / "home.ls").read_text().replace("/1000/1000/.bashrc", "/1000/1000\u00b2/.bashrc"),
             "cannot parse"),
            ("home.ls", (check / "home.ls").read_text().replace("/1000/1000/.bashrc", "/1000/" + "1" * 5000 + "/.bashrc"),
             "cannot parse"),
            ("home.stat", home.replace("User:  1000", "User:  \u0661\u0660\u0660\u0660"), "/home/workload"),
            ("home.stat", home.replace("User:  1000", "User:  " + "1" * 5000), "/home/workload"),
            ("add-1.stat", (check / "add-1.stat").read_text().replace("Mode:  0755", "Mode:  0644"), "lane-probe"),
            ("add-1.stat", (check / "add-1.stat").read_text().replace("User:     0", "User:  1000"), "lane-probe"),
            ("add-1.data", "other bytes", "lane-probe"),
            ("dumpe2fs", (check / "dumpe2fs").read_text().replace(uuid, "0" * 8 + uuid[8:]), "UUID"),
            ("dumpe2fs", (check / "dumpe2fs").read_text().replace("4096", "1024"), "block size"),
            ("mke2fs-version", "unknown\n", "mke2fs"),
        ]
        for name, content, needle in cases:
            with changed(check / name, content):
                detail = refusal("check", API["check_image"], check, facts, uuid)
                assert needle in detail, (name, needle, detail)
        with changed(check / "home.stat.err", f"{BANNER}/home/workload: File not found by ext2_lookup \n"):
            with changed(check / "home.stat", ""):
                assert "missing" in refusal("check", API["check_image"], check, facts, uuid)
        with changed(check / "opt.ls", ""), changed(check / "opt.ls.err", f"{BANNER}/opt: File not found by ext2_lookup \n"):
            assert API["check_image"](check, facts, uuid)["mke2fs_version"] == "1.47.0"
        with changed(check / "opt.ls", "/2/040755/0/0/./\n"):
            assert "cannot parse" in refusal("check", API["check_image"], check, facts, uuid)
        with patched("MAX_ADD_BYTES", 2):
            assert "larger than 2 bytes" in refusal("check", API["check_image"], check, facts, uuid)
        with oversize(check / "dumpe2fs", TOOL["MAX_OUTPUT_BYTES"] + 1):
            detail = refusal("check", API["check_image"], check, facts, uuid)
        assert "check/dumpe2fs" in detail and f"larger than {TOOL['MAX_OUTPUT_BYTES']} bytes" in detail, detail
        (check / "group").unlink()
        assert "/etc/group" in refusal("check", API["check_image"], check, facts, uuid)
        (check / "group").symlink_to(check / "passwd")
        assert "cannot read /etc/group" in refusal("check", API["check_image"], check, facts, uuid)
        (check / "group").unlink()
        os.mkfifo(check / "group")
        assert "not a regular file" in refusal("check", API["check_image"], check, facts, uuid)


WORST_STAT = """
import runpy, sys
from pathlib import Path
api = runpy.run_path(sys.argv[1])
try:
    api["read_stat"](Path(sys.argv[2]), sys.argv[3], "/tmp")
except api["Refused"] as error:
    print(error.step)
"""


def a_worst_case_stat_read_back_is_refused_in_seconds():
    with scratch() as root:
        size = TOOL["MAX_OUTPUT_BYTES"]
        for name, head, text in [("pairs.stat", "", "Type: regular    Mode:  0644   "),
                                 ("users.stat", "Inode: 12   Type: directory    Mode:  01777   Flags: 0x80000\n",
                                  "User:  1000   ")]:
            (root / name).write_text(head + text * ((size - len(head)) // len(text)))
            (root / f"{name}.err").write_text(BANNER)
            started = time.monotonic()
            done = subprocess.run([sys.executable, "-c", WORST_STAT, str(SOURCE), str(root), name],
                                  capture_output=True, text=True, timeout=10, stdin=subprocess.DEVNULL)
            assert done.stdout == "check\n", (name, done.stdout, done.stderr[-500:], time.monotonic() - started)


def judge_accounts_accepts_exactly_the_contract():
    API["judge_accounts"](USERS + WORKLOAD_USER, GROUP + WORKLOAD_GROUP)
    API["judge_accounts"](USERS + WORKLOAD_USER.rstrip(b"\n"), GROUP + WORKLOAD_GROUP.rstrip(b"\n"))
    for users, group, needle in [
        (USERS, GROUP + WORKLOAD_GROUP, "no line with uid 1000"),
        (USERS + WORKLOAD_USER + b"workload:x:1001:1001::/home/workload:/bin/bash\n", GROUP + WORKLOAD_GROUP, "line 4"),
        (USERS + WORKLOAD_USER + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "lines 3, 4"),
        (USERS + b"-workload::::::\n" + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "line 3"),
        (USERS + b"+alice:x:1001:1001::/home/alice:/bin/sh\n" + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "line 3"),
        (USERS + WORKLOAD_USER, GROUP + WORKLOAD_GROUP + WORKLOAD_GROUP, "lines 4, 5"),
        (USERS + WORKLOAD_USER, GROUP + b"workload:x:1001:\n" + WORKLOAD_GROUP, "line 4"),
        (b"\xff\n" + USERS + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "line 1"),
        (USERS + b" workload:x:1001:1001::/home/w:/bin/sh\n" + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "line 3 names"),
        (USERS + WORKLOAD_USER, GROUP + b"\tworkload:x:1001:\n" + WORKLOAD_GROUP, "line 4 names"),
        (USERS + b" +alice:x:1001:1001::/home/alice:/bin/sh\n" + WORKLOAD_USER, GROUP + WORKLOAD_GROUP, "line 3 is malformed"),
    ]:
        assert needle in refusal("check", API["judge_accounts"], users, group), needle


def build_runs_export_then_filesystem_in_the_pinned_base():
    with scratch() as root:
        probe = root / "probe"
        probe.write_bytes(b"probe")
        fake = FakeDocker()
        code, out, err, work, images = built(root, fake, adds=[f"{probe}:/usr/bin/lane-probe:0755"])
        assert code == 0, err
        result = json.loads(out)
        content = (work / "content.jsonl").read_bytes()
        digest = sha(content)
        uuid = f"{digest[:8]}-{digest[8:12]}-{digest[12:16]}-{digest[16:20]}-{digest[20:32]}"
        script = fake.calls[5][-1]
        assert fake.calls == [
            ["docker", "image", "inspect", REF],
            ["docker", "image", "inspect", "--platform", "linux/arm64", REF],
            ["docker", "create", "--pull", "never", "--platform", "linux/arm64", "--network", "none", REF, "/bin/true"],
            ["docker", "export", "-o", str(work / "base.tar"), CONTAINER],
            ["docker", "rm", CONTAINER],
            ["docker", "run", "--rm", "--pull", "never", "--network", "none", "--platform", "linux/arm64",
             "--user", "0:0", "-v", f"{work}:/w", REF, "sh", "-c", script],
            ["chflags", "-R", "uchg", result["image_dir"]],
        ], fake.calls
        lines = script.splitlines()
        assert lines[0] == "set -eu" and lines[1].startswith("test -f /w/stage.tar || ") and "exit 3" in lines[1]
        [extract] = [number for number, line in enumerate(lines) if line.startswith("tar ")]
        tree = lines[extract].removeprefix("tar --numeric-owner -xpf /w/stage.tar -C ")
        assert tree.startswith("/") and " " not in tree, lines[extract]
        assert posixpath.normpath(tree) != "/w" and not posixpath.normpath(tree).startswith("/w/"), tree
        [mkfs] = [number for number, line in enumerate(lines) if line.startswith("mke2fs -q ")]
        assert lines[mkfs] == (f"mke2fs -q -t ext4 -b 4096 -d {tree} -U {uuid} -E hash_seed={uuid},root_owner=0:0 "
                               f"/w/rootfs.raw {SIZE_MIB}M"), lines[mkfs]
        [check] = [number for number, line in enumerate(lines) if "e2fsck -fn /w/rootfs.raw" in line]
        assert lines[check] == ("if e2fsck -fn /w/rootfs.raw >/w/check/e2fsck.out 2>&1; then echo 0; else echo $?; fi "
                                ">/w/check/e2fsck.code"), lines[check]
        assert extract < mkfs < check
        probes = re.findall(r"^debugfs -R '([^']+)' /w/rootfs.raw >/w/check/(\S+) 2>/w/check/\2\.err$", script, re.M)
        assert sorted(probes) == sorted([
            ("cat /etc/passwd", "passwd"), ("cat /etc/group", "group"), ("stat /home/workload", "home.stat"),
            ("ls -p /home/workload", "home.ls"), ("stat /tmp", "tmp.stat"), ("ls -p /opt", "opt.ls"),
            ("ls -p /usr/local/bin", "usr-local-bin.ls"),
            ("stat /usr/bin/lane-probe", "add-1.stat"), ("cat /usr/bin/lane-probe", "add-1.data")]), probes
        assert min(number for number, line in enumerate(lines) if line.startswith("debugfs ")) > check
        for argv in fake.calls:
            assert not [a for a in argv if a.startswith(("--network=", "--net=")) or a == "--net"], argv
            assert [argv[i + 1] for i, a in enumerate(argv) if a == "--network"] in ([], ["none"]), argv
        assert fake.envs == [ENV] * len(fake.calls)


def a_base_without_a_digest_refuses_before_docker():
    with scratch() as root:
        bad = ["debian:bookworm-slim", f"debian@sha256:{DIGEST[:63]}", f"debian@sha256:{DIGEST.upper()}",
               f"debian:bookworm-slim@sha512:{DIGEST}", f"@sha256:{DIGEST}", f"debian@sha256:{DIGEST}0"]
        for base in bad:
            fake = FakeDocker()
            code, out, err, work, images = built(root, fake, base=base)
            assert (code, out, fake.calls) == (1, "", []), (base, err)
            assert err.startswith("root-image: refused base: ") and not work.exists(), err
        for extra, step in [(("--size-mib", "0"), "size"), (("--add", "/x:/usr/bin/x:4755"), "add"),
                            (("--platform", "linux"), "platform"), (("--source-date-epoch", "-1"), "source date epoch")]:
            fake = FakeDocker()
            code, out, err, work, images = built(root, fake, extra=extra)
            assert (code, fake.calls) == (1, []) and err.startswith(f"root-image: refused {step}: "), err
        assert API["parse_base"](REF) == API["Base"](REF, f"sha256:{DIGEST}")


def a_base_that_is_not_local_or_not_pinned_refuses_before_create():
    with scratch() as root:
        fake = FakeDocker()
        fake.missing = True
        code, out, err, work, images = built(root, fake, work_name="missing")
        assert code == 1 and f"docker pull {REF}" in err and err.startswith("root-image: refused export: "), err
        assert [argv[:2] for argv in fake.calls] == [["docker", "image"]]
        assert work.is_dir()
        fake = FakeDocker()
        fake.repo_digests = [f"debian@sha256:{'0' * 64}"]
        code, out, err, work, images = built(root, fake, work_name="unpinned")
        assert code == 1 and DIGEST in err and ["docker", "create"] not in [argv[:2] for argv in fake.calls], err
        fake = FakeDocker()
        fake.descriptor = {**fake.descriptor, "digest": f"sha256:{'0' * 64}"}
        code, out, err, work, images = built(root, fake, work_name="other-index")
        assert code == 1 and DIGEST in err and ["docker", "create"] not in [argv[:2] for argv in fake.calls], err
        fake = FakeDocker()
        fake.descriptor = None
        code, out, err, work, images = built(root, fake, work_name="classic-store")
        assert code == 0, err
        manifest = json.loads((Path(json.loads(out)["image_dir"]) / "manifest.json").read_text())
        assert manifest["base"]["digest_form"] == "unknown" and manifest["base"]["digest"] == f"sha256:{DIGEST}"
        for attribute, value, needle in [("inspect_out", "[]", "no image record"), ("create_code", 125, "create"),
                                         ("create_out", "WARNING: the image has no linux/arm64 variant\n", "create"),
                                         ("export_code", 1, "export"), ("rm_code", 1, "remains")]:
            fake = FakeDocker()
            setattr(fake, attribute, value)
            code, out, err, work, images = built(root, fake, work_name=attribute)
            assert code == 1 and err.startswith("root-image: refused export: ") and needle in err, err
            assert attribute != "export_code" or fake.calls[-1] == ["docker", "rm", CONTAINER], fake.calls
        fake = FakeDocker()
        fake.platform = {"Os": "", "Architecture": ""}
        code, out, err, work, images = built(root, fake, work_name="platform")
        assert code == 1 and "linux/arm64" in err and ["docker", "create"] not in [argv[:2] for argv in fake.calls], err


def e2fsck_failure_refuses():
    with scratch() as root:
        fake = FakeDocker()
        fake.e2fsck = "4"
        code, out, err, work, images = built(root, fake, work_name="unclean")
        assert code == 1 and err.startswith("root-image: refused filesystem: ") and "e2fsck" in err, err
        assert "illegal block" in err and list(images.iterdir()) == [] and work.is_dir()
        fake = FakeDocker()
        fake.after = lambda work: (work / "check" / "e2fsck.code").write_text("0" + " " * 64 + "\n")
        code, out, err, work, images = built(root, fake, work_name="long-code")
        assert code == 1 and err.startswith("root-image: refused filesystem: ") and "larger than 64 bytes" in err, err
        fake = FakeDocker()
        fake.e2fsck = "4"
        fake.after = lambda work: os.truncate(work / "check" / "e2fsck.out", TOOL["MAX_OUTPUT_BYTES"] + 1)
        code, out, err, work, images = built(root, fake, work_name="long-out")
        assert code == 1 and f"larger than {TOOL['MAX_OUTPUT_BYTES']} bytes" in err, err
        fake = FakeDocker()
        fake.unshared = True
        code, out, err, work, images = built(root, fake, work_name="unshared")
        assert code == 1 and "Docker does not share the work root" in err, err
        fake = FakeDocker()
        fake.run_code = 125
        code, out, err, work, images = built(root, fake, work_name="no-container")
        assert code == 1 and err.startswith("root-image: refused filesystem: ") and "125" in err, err
        assert list(images.iterdir()) == []


def manifest_has_sorted_keys_and_files_equal_sha256sums():
    with scratch() as root:
        code, out, err, work, images = built(root, FakeDocker())
        assert code == 0 and out.count("\n") == 1 and err == "", err
        result = json.loads(out)
        image = Path(result["image_dir"])
        text = (image / "manifest.json").read_bytes()
        manifest = json.loads(text)
        assert text == (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode()
        content = (image / "content.jsonl").read_bytes()
        assert content == (work / "content.jsonl").read_bytes()
        digest = sha(content)
        uuid = f"{digest[:8]}-{digest[8:12]}-{digest[12:16]}-{digest[16:20]}-{digest[20:32]}"
        assert result == {"image_dir": str(images / sha(text)[:16]), "image_id16": sha(text)[:16],
                          "manifest_sha256": sha(text), "rootfs_sha256": sha(ROOTFS),
                          "content_sha256": digest, "kind": "product"}
        sums = (image / "SHA256SUMS").read_text()
        assert manifest["files"] == {"content.jsonl": digest, "rootfs.raw": sha(ROOTFS)}
        assert sums == "".join(f"{value}  {name}\n" for name, value in sorted(manifest["files"].items()))
        assert manifest["sha256sums_sha256"] == sha(sums.encode())
        assert manifest["base"] == {"reference": REF, "digest": f"sha256:{DIGEST}", "digest_form": "index",
                                    "platform": "linux/arm64", "platform_manifest_digest": f"sha256:{PLATFORM_DIGEST}",
                                    "image_id": f"sha256:{DIGEST}", "debian_version": "12.15"}
        assert manifest["workload"] == {"user": "workload", "uid": 1000, "gid": 1000, "home": "/home/workload",
                                        "shell": "/bin/bash"}
        assert manifest["fs"] == {"type": "ext4", "block_size": 4096, "size_mib": SIZE_MIB, "uuid": uuid,
                                  "mke2fs_version": "1.47.0", "e2fsck": "clean", "features": FEATURES.split()}
        assert (manifest["image_format"], manifest["task"], manifest["kind"]) == (
            "plasmosome-root-image/1", "plasmosome-2s1x", "product")
        assert (manifest["created_unix"], manifest["source_date_epoch"]) == (NOW, EPOCH)
        assert manifest["content_sha256"] == digest and manifest["added"] == []
        assert manifest["limits"] and all(isinstance(line, str) for line in manifest["limits"])
        assert any("mke2fs, e2fsck, dumpe2fs and debugfs" in line and "checks itself" in line
                   for line in manifest["limits"]), manifest["limits"]
        assert any("content.jsonl" in line and "stage tar" in line for line in manifest["limits"]), manifest["limits"]
        assert any("tar and sh" in line and "checks itself" in line for line in manifest["limits"]), manifest["limits"]
        assert manifest["tool_sha256"] == sha(SOURCE.read_bytes())
        assert sorted(os.listdir(image)) == ["SHA256SUMS", "content.jsonl", "manifest.json", "rootfs.raw"]
        assert {stat.S_IMODE(os.stat(image / name).st_mode) for name in os.listdir(image)} == {0o444}
        assert stat.S_IMODE(os.stat(image).st_mode) == 0o555
        assert [path.name for path in images.iterdir()] == [image.name]


def staged_image(images):
    stage_dir = images / ".stage-test"
    stage_dir.mkdir(mode=0o700)
    (stage_dir / "rootfs.raw").write_bytes(ROOTFS)
    (stage_dir / "manifest.json").write_bytes(b'{\n  "kind": "product"\n}\n')
    return stage_dir, sha(b'{\n  "kind": "product"\n}\n')


def an_existing_image_id_refuses_and_nothing_is_deleted():
    with scratch() as root:
        images = root / "images"
        images.mkdir()
        stage_dir, manifest_sha256 = staged_image(images)
        target = images / manifest_sha256[:16]
        target.mkdir()
        (target / "rootfs.raw").write_bytes(b"the published image")
        calls = []
        detail = refusal("place", API["place"], lambda argv, env: calls.append(argv), ENV, images, stage_dir)
        assert str(target) in detail and calls == []
        assert (target / "rootfs.raw").read_bytes() == b"the published image"
        assert sorted(os.listdir(target)) == ["rootfs.raw"] and stage_dir.is_dir()
        (target / "rootfs.raw").unlink()
        refusal("place", API["place"], lambda argv, env: calls.append(argv), ENV, images, stage_dir)
        assert target.is_dir() and calls == [] and (stage_dir / "rootfs.raw").read_bytes() == ROOTFS
        target.rmdir()
        target.symlink_to(root / "elsewhere")
        refusal("place", API["place"], lambda argv, env: calls.append(argv), ENV, images, stage_dir)
        assert target.is_symlink() and calls == []


def placement_renames_then_sets_uchg():
    with scratch() as root:
        images = root / "images"
        images.mkdir()
        stage_dir, manifest_sha256 = staged_image(images)
        target = images / manifest_sha256[:16]
        seen = []

        def run(argv, env):
            seen.append((argv, env, target.is_dir(), stage_dir.exists(), stat.S_IMODE(os.stat(target).st_mode)))
            return Result(0, "", "")

        assert API["place"](run, ENV, images, stage_dir) == (target, manifest_sha256[:16], manifest_sha256)
        assert seen == [(["chflags", "-R", "uchg", str(target)], ENV, True, False, 0o555)]
        assert (target / "rootfs.raw").read_bytes() == ROOTFS
        other, _ = staged_image(images)
        (other / "manifest.json").write_bytes(b"{}\n")
        detail = refusal("place", API["place"], lambda argv, env: Result(1, "", "chflags: denied\n"), ENV, images, other)
        assert "denied" in detail and (images / sha(b"{}\n")[:16]).is_dir()


def failure_before_placement_removes_the_stage_and_keeps_the_work_root():
    with scratch() as root:
        def failing(run, env, images_root, stage_dir):
            raise Refused("place", "injected")

        with patched("place", failing):
            code, out, err, work, images = built(root, FakeDocker(), work_name="injected")
        assert code == 1 and err == "root-image: refused place: injected\n", err
        assert list(images.iterdir()) == []
        assert {"base.tar", "stage.tar", "content.jsonl", "rootfs.raw"} <= set(os.listdir(work))
        class Interrupted:
            def __getattr__(self, attribute):
                if attribute == "dumps":
                    def dumps(*args, **kwargs):
                        raise KeyboardInterrupt
                    return dumps
                return getattr(json, attribute)

        with patched("json", Interrupted()):
            try:
                built(root, FakeDocker(), work_name="interrupted")
                raise AssertionError("the interrupt did not propagate")
            except KeyboardInterrupt:
                pass
        assert list(images.iterdir()) == []
        fake = FakeDocker()
        fake.rootfs = None
        code, out, err, work, images = built(root, fake, work_name="no-rootfs")
        assert code == 1 and err.startswith("root-image: refused publish: ") and "rootfs.raw" in err, err
        assert list(images.iterdir()) == [] and (work / "stage.tar").is_file()


def reads_from_the_base_are_bounded():
    with scratch() as root:
        assert "4096" in refusal("stage", stage_with, root, limit=4096)
        fake = FakeDocker({"usr/share": d(), "usr/share/big": f(bytes(SIZE_MIB << 20))})
        code, out, err, work, images = built(root, fake, work_name="big-export")
        assert code == 1 and err.startswith("root-image: refused stage: ") and str(SIZE_MIB << 20) in err, err
        assert "/etc/passwd" in refusal("stage", stage_with, root, {"etc/passwd": f(USERS + b"x" * (1 << 20))})
        with patched("MAX_OUTPUT_BYTES", 8):
            detail = refusal("command", API["run_command"], [sys.executable, "-c", "print('x' * 9)"], dict(os.environ))
        assert "8 bytes" in detail
        result = API["run_command"]([sys.executable, "-c", "import sys; print('out'); sys.exit('err')"], dict(os.environ))
        assert result == Result(1, "out\n", "err\n")
        assert "not installed" in refusal("command", API["run_command"], [str(root / "no-such-program")], {})
        with patched("COMMAND_TIMEOUT", 0.1):
            detail = refusal("command", API["run_command"], [sys.executable, "-c", "import time; time.sleep(5)"], {})
        assert "longer than" in detail
        reader = API["LimitedReader"](io.BytesIO(b"abc"), 2, "the export")
        assert "the export is larger than 2 bytes" in refusal("stage", reader.read)

        class Recorded(io.BytesIO):
            def read(self, size=-1):
                asked.append(size)
                return super().read(size)

        asked = []
        assert API["LimitedReader"](Recorded(b"ab"), 2, "the export").read(1 << 29) == b"ab"
        assert "larger than 2 bytes" in refusal("stage", API["LimitedReader"](Recorded(b"x" * 10), 2, "the export").read,
                                                1 << 29)
        assert max(asked) <= 3, asked


def work_and_images_roots_are_checked_before_docker():
    with scratch() as root:
        fake = FakeDocker()
        (root / "taken").mkdir()
        code, out, err, work, images = built(root, fake, work_name="taken")
        assert code == 1 and fake.calls == [] and err.startswith("root-image: refused work root: "), err
        fake = FakeDocker()
        argv = ["build", "--base", REF, "--work-root", str(root / "fresh"), "--images-root", str(root / "none")]
        err = io.StringIO()
        with redirect_stderr(err):
            assert API["main"](argv, run=fake, now=lambda: NOW, environ=ENVIRON) == 1
        assert fake.calls == [] and err.getvalue().startswith("root-image: refused images root: ")
        assert not (root / "fresh").exists()
        assert "cannot create" in refusal("work root", API["make_work_root"], root / "no-parent" / "work")
        made = API["make_work_root"](root / "made")
        assert made == root / "made" and stat.S_IMODE(os.stat(made).st_mode) == 0o700
        assert API["docker_env"](ENVIRON) == ENV


def a_member_under_a_link_or_a_file_refuses():
    with scratch() as root:
        for changes, name in [
            ({"zz-etc": s("etc"), "zz-etc/hostname": f(b"4f1c0a9e2b7d\n")}, "/zz-etc/hostname"),
            ({"zz-tmp": s("tmp"), "zz-tmp/planted": f(b"planted")}, "/zz-tmp/planted"),
            ({"zz-home": s("home"), "zz-home/workload/.bash_aliases": f(b"alias")}, "/zz-home/workload/.bash_aliases"),
            ({"zz-dev": s("dev"), "zz-dev/console": f(mode=0o755)}, "/zz-dev/console"),
            ({"etc/passwd/x": f()}, "/etc/passwd/x"),
            ({"var/cache/x": f()}, "/var/cache/x"),
        ]:
            detail = refusal("stage", stage_with, root, changes)
            assert name in detail and "earlier directory" in detail, detail
        assert "normalised" in refusal("stage", stage_with, root, {".": d(0o777)})


def planted(work, name, target):
    (work / name).unlink()
    (work / name).symlink_to(target)


def files_the_container_leaves_in_the_work_root_are_not_trusted():
    with scratch() as root:
        host = root / "host-file"
        host.write_bytes(ROOTFS)
        for number, (after, step, needle) in enumerate([
            (lambda work: planted(work, "rootfs.raw", host), "publish", "rootfs.raw"),
            (lambda work: planted(work, "rootfs.raw", "/dev/zero"), "publish", "rootfs.raw"),
            (lambda work: (work / "rootfs.raw").write_bytes(ROOTFS[:-1]), "publish", str(SIZE_MIB << 20)),
            (lambda work: (work / "rootfs.raw").write_bytes(ROOTFS + b"x"), "publish", str(SIZE_MIB << 20)),
            (lambda work: (shutil.move(work / "check", work / "elsewhere"), (work / "check").symlink_to("elsewhere")),
             "filesystem", "check"),
            (lambda work: shutil.rmtree(work / "check"), "filesystem", "check"),
        ]):
            fake = FakeDocker()
            fake.after = after
            code, out, err, work, images = built(root, fake, work_name=f"planted-{number}")
            assert code == 1 and err.startswith(f"root-image: refused {step}: ") and needle in err, (number, err)
            assert [name for name in os.listdir(images) if not name.startswith(".")] == [], os.listdir(images)
        fake = FakeDocker()
        fake.after = lambda work: (work / "content.jsonl").write_bytes(b'{"path":"/forged"}\n')
        code, out, err, work, images = built(root, fake, work_name="forged")
        assert code == 0, err
        result = json.loads(out)
        image = Path(result["image_dir"])
        manifest = json.loads((image / "manifest.json").read_text())
        placed = (image / "content.jsonl").read_bytes()
        assert sha(placed) == result["content_sha256"] == manifest["content_sha256"] == manifest["files"]["content.jsonl"]
        assert b"/forged" not in placed and b'"/etc/hostname"' in placed
        assert (image / "rootfs.raw").read_bytes() == ROOTFS and manifest["files"]["rootfs.raw"] == sha(ROOTFS)


class Skipped(Exception):
    """A test that cannot run on this machine, with the reason the run prints."""


def hashed(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def the_placed_rootfs_keeps_its_holes():
    with scratch() as root:
        size, data, source = 128 << 20, b"superblock".ljust(4096, b"\1"), root / "rootfs.raw"
        with open(source, "wb") as out:
            out.truncate(size)
            for offset in (0, 65 << 20, size - 4096):
                os.pwrite(out.fileno(), data, offset)
        allocated = os.stat(source).st_blocks * 512
        if allocated >= size // 2:
            raise Skipped(f"this filesystem allocates {allocated} bytes for a {size}-byte file with holes")
        (root / "stage").mkdir()
        digest = API["copy_rootfs"](root, root / "stage", size)
        copied = root / "stage" / "rootfs.raw"
        assert [hashed(source), hashed(copied)] == [digest, digest], digest
        placed = os.stat(copied).st_blocks * 512
        assert os.stat(copied).st_size == size and placed <= allocated + (1 << 20) and placed < 16 << 20, (placed, allocated)


def the_rootfs_copy_reads_no_more_than_its_size():
    with scratch() as root:
        size = 1 << 20
        with open(root / "rootfs.raw", "wb") as out:
            out.truncate(64 << 20)
        (root / "stage").mkdir()
        sizes, opened = [], TOOL["open_regular"]

        class Counted:
            def __init__(self, handle):
                self.handle = handle

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                self.handle.close()

            def read(self, wanted=-1):
                data = self.handle.read(wanted)
                sizes.append(len(data))
                return data

        with patched("open_regular", lambda *args: Counted(opened(*args))):
            detail = refusal("publish", API["copy_rootfs"], root, root / "stage", size)
        assert str(size) in detail and sum(sizes) == size + 1, (detail, sum(sizes))


class Failing:
    """A module whose one named function raises the OSError a full disk or a denied write gives."""

    def __init__(self, module, name):
        self.module, self.name = module, name

    def __getattr__(self, attribute):
        if attribute == self.name:
            def fail(*args, **kwargs):
                raise PermissionError(13, "Permission denied")
            return fail
        return getattr(self.module, attribute)


class FullDisk(io.RawIOBase):
    def tell(self):
        return 0

    def write(self, data):
        raise OSError(28, "No space left on device")


class Unreadable:
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def read(self, size=-1):
        raise OSError(5, "Input/output error")


def tool_through(link, fake, root, work_name):
    linked = runpy.run_path(str(link))
    argv = ["build", "--base", REF, "--work-root", str(root / work_name), "--images-root", str(root / "images"),
            "--source-date-epoch", str(EPOCH), "--size-mib", str(SIZE_MIB)]
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = linked["main"](argv, run=fake, now=lambda: NOW, environ=ENVIRON)
    return code, out.getvalue(), err.getvalue()


def every_other_failure_prints_the_refused_line():
    with scratch() as root:
        real_tarfile = tarfile

        class FullTarfile:
            def __getattr__(self, attribute):
                return getattr(real_tarfile, attribute)

            def open(self, name=None, mode="r", fileobj=None, **kwargs):
                if mode == "w":
                    return real_tarfile.open(fileobj=FullDisk(), mode="w", **kwargs)
                return real_tarfile.open(name, mode, fileobj, **kwargs)

        with patched("tarfile", FullTarfile()):
            code, out, err, work, images = built(root, FakeDocker(), work_name="full-stage")
        assert code == 1 and err.startswith("root-image: refused stage: ") and "No space left" in err, err
        fake = FakeDocker()
        fake.exported = lambda work: (work / "content.jsonl").mkdir()
        code, out, err, work, images = built(root, fake, work_name="content-in-the-way")
        assert code == 1 and err.startswith("root-image: refused stage: ") and "content.jsonl" in err, err
        docker = root / "docker"
        docker.write_bytes(b"#!/bin/sh\n")
        docker.chmod(0o644)
        assert "cannot run" in refusal("command", API["run_command"], [str(docker), "version"], {})
        for attribute, value in [("inspect_out", json.dumps([{"Id": "x", "RepoDigests": 5}])),
                                 ("inspect_out", json.dumps([{"RepoDigests": [f"debian@sha256:{DIGEST}"],
                                                              "Descriptor": "sha256"}])),
                                 ("descriptor", {"mediaType": ["index"], "digest": f"sha256:{DIGEST}"}),
                                 ("platform_descriptor", "sha256")]:
            fake = FakeDocker()
            setattr(fake, attribute, value)
            code, out, err, work, images = built(root, fake, work_name=f"shape-{len(os.listdir(root))}")
            assert code == 1 and err.startswith("root-image: refused export: ") and "record" in err, (attribute, err)
        for mtime in ("1e400", "nan"):
            base = root / f"base-{mtime}.tar"
            write_tar(base, DEBIAN)
            with tarfile.open(base, "a", format=tarfile.PAX_FORMAT) as out:
                info = tarfile.TarInfo("etc/far-future")
                info.pax_headers = {"mtime": mtime}
                out.addfile(info, io.BytesIO(b""))
            detail = refusal("stage", API["stage"], base, root / f"stage-{mtime}.tar", [], EPOCH, LIMIT)
            assert "/etc/far-future" in detail and "mtime" in detail, detail
        with patched("open_regular", lambda *args: Unreadable()):
            assert "Input/output error" in refusal("check", API["read_bounded"], root / "x", "check", 8, "x")
            assert "Input/output error" in refusal("check", API["hash_bounded"], root / "x", "check", 8, "x")
        link = root / "bin" / "root-image"
        link.parent.mkdir()
        link.symlink_to(SOURCE)
        code, out, err = tool_through(link, FakeDocker(), root, "through-a-link")
        assert code == 0, err
        manifest = json.loads((Path(json.loads(out)["image_dir"]) / "manifest.json").read_text())
        assert manifest["tool_sha256"] == sha(SOURCE.read_bytes())


def failures_of_the_os_print_the_refused_line():
    with scratch() as root:
        fake = FakeDocker()
        images = root / "read-only"
        images.mkdir(mode=0o555)
        argv = ["build", "--base", REF, "--work-root", str(root / "work"), "--images-root", str(images)]
        err = io.StringIO()
        with redirect_stderr(err):
            assert API["main"](argv, run=fake, now=lambda: NOW, environ=ENVIRON) == 1
        assert fake.calls == [] and err.getvalue().startswith("root-image: refused images root: "), err.getvalue()
        with patched("tempfile", Failing(tempfile, "mkdtemp")):
            code, out, err, work, images = built(root, FakeDocker(), work_name="no-stage")
        assert code == 1 and err.startswith("root-image: refused publish: ") and "denied" in err, err
        work = root / "laid-out"
        work.mkdir()
        (work / "rootfs.raw").write_bytes(ROOTFS)
        with patched("os", Failing(os, "chmod")):
            detail = refusal("publish", API["write_stage"], images, work, b"{}\n", len(ROOTFS), {})
        assert "cannot lay out the image" in detail and [n for n in os.listdir(images) if n.startswith(".")] == []
        for name, needle in (("rename", "cannot rename"), ("chmod", "in place")):
            places = root / f"images-{name}"
            places.mkdir()
            stage_dir, _ = staged_image(places)
            with patched("os", Failing(os, name)):
                detail = refusal("place", API["place"], lambda argv, env: Result(0, "", ""), ENV, places, stage_dir)
            assert needle in detail and "denied" in detail, detail
        fake = FakeDocker()
        fake.inspect_out = "[" * 1_000_000 + "]" * 1_000_000
        code, out, err, work, images = built(root, fake, work_name="deep")
        assert code == 1 and err.startswith("root-image: refused export: ") and "no image record" in err, err


TESTS = [
    stage_adds_the_workload_account_home_and_tmp,
    an_existing_workload_name_or_id_1000_refuses,
    entries_under_opt_or_usr_local_bin_refuse,
    the_product_image_has_nothing_under_opt_or_usr_local_bin,
    dev_proc_sys_are_emptied_and_dockerenv_dropped,
    a_malformed_base_member_refuses,
    network_files_are_fixed,
    a_member_naming_plasmosome_refuses,
    add_places_a_root_owned_file_and_refuses_bad_destinations,
    the_kind_is_test_exactly_when_add_is_given,
    content_listing_is_sorted_and_stable,
    docker_create_times_do_not_reach_the_stage,
    contract_check_reads_debugfs_output,
    judge_accounts_accepts_exactly_the_contract,
    a_worst_case_stat_read_back_is_refused_in_seconds,
    build_runs_export_then_filesystem_in_the_pinned_base,
    a_base_without_a_digest_refuses_before_docker,
    a_base_that_is_not_local_or_not_pinned_refuses_before_create,
    e2fsck_failure_refuses,
    manifest_has_sorted_keys_and_files_equal_sha256sums,
    an_existing_image_id_refuses_and_nothing_is_deleted,
    placement_renames_then_sets_uchg,
    failure_before_placement_removes_the_stage_and_keeps_the_work_root,
    reads_from_the_base_are_bounded,
    work_and_images_roots_are_checked_before_docker,
    a_member_under_a_link_or_a_file_refuses,
    files_the_container_leaves_in_the_work_root_are_not_trusted,
    failures_of_the_os_print_the_refused_line,
    every_other_failure_prints_the_refused_line,
    the_placed_rootfs_keeps_its_holes,
    the_rootfs_copy_reads_no_more_than_its_size,
]


def main(names):
    known = {test.__name__ for test in TESTS}
    unknown = sorted(set(names) - known)
    if unknown:
        print(f"unknown tests: {', '.join(unknown)}")
        return 2
    selected = [test for test in TESTS if not names or test.__name__ in names]
    failed, skipped = [], []
    for test in selected:
        try:
            test()
        except Skipped as reason:
            skipped.append(test.__name__)
            print(f"skip {test.__name__}: {reason}")
        except Exception as error:
            failed.append(test.__name__)
            where = traceback.extract_tb(error.__traceback__)[-1]
            print(f"FAIL {test.__name__}: {type(error).__name__}: {error} (line {where.lineno})")
        else:
            print(f"ok   {test.__name__}")
    if failed:
        print(f"FAILED: {len(failed)} of {len(selected)} root image tests")
        return 1
    ran = f"{len(selected) - len(skipped)} root image tests" + (f", {len(skipped)} skipped" if skipped else "")
    print(f"PASS: {ran}: contract, refusals, pinned docker sequence, placement")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
