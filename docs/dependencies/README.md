# Dependency records

Each third-party crate the workspace depends on directly has a review record here, one TOML file
named `<crate>.toml`. A record says why we use the crate, roughly what writing the part we use
would cost, and what the crate brings with it: the packages in its closure, the code that runs at
build time, the macros that put `unsafe` into our code, and any native code. The record and the
rules around it are defined in
[spec 026, section 1](../specs/026-security-standard.md#1-dependency-admission). This file says
how to fill one in, so that two people derive the same facts.

Three fields are derived: `range`, `brings` and `build_time_code`. They come from
`cargo metadata` by the procedure below, and nobody writes them by hand. The other fields are
written after reading the crate's code. Until the owner answers spec 026's Q4, every record's
verdict is `kept-pending-owner`.

## The record

This example is made up. The real records sit beside this file.

```toml
name = "example"
range = "0.4"
reason = "What it does for us, in a sentence or two."
write_it_instead.lines = 900
write_it_instead.how = "What was counted, and how that became the estimate."
exception = ""
brings = ["example-macros@0.4", "example-sys@0.4"]
build_time_code = ["example-macros@0.4 proc macro", "example-sys@0.4 build script"]
expands_unsafe = ["example-macros@0.4 derive(Pod) expands an unsafe impl"]
native_code = ["example-sys@0.4 compiles its vendored C with cc"]
verdict = "kept-pending-owner"
```

- Every one of these ten keys is required, and no other key is allowed. What each one means is
  spec 026's table; it is not repeated here.
- `write_it_instead` is a table of two keys: `lines`, an integer, and `how`, a string.
- `brings` and `build_time_code` are sorted arrays, copied from the derivation.
- `expands_unsafe` and `native_code` are the string `"none"`, or a non-empty array of strings.
  Each string reads `<name>@<range> <what>`, and names the crate or a package in its `brings`.
- `exception` is the empty string unless the owner approved an exception. Then it holds the
  argument and a link to where the approval was given.
- `verdict` is `admitted`, `kept-pending-owner` or `replace`.
- Features, and which of our crates use a dependency, are not kept in the record. Read them from
  `cargo tree`.

Long strings may use TOML's multi-line form, `"""\`, with a backslash ending each line.

## Deriving `range`, `brings` and `build_time_code`

Run these from the repository root, at the commit the change branches from. `cargo metadata`
runs no build script or proc macro, and `--locked` makes it fail rather than re-resolve, so the
facts are the lockfile's.

```shell
cargo metadata --locked --all-features --format-version 1 > all.json
cargo metadata --locked --all-features --format-version 1 \
  --filter-platform aarch64-apple-darwin --filter-platform x86_64-unknown-linux-gnu > hosts.json
python3 record-facts.py all.json hosts.json <crate> [<range>]
```

`record-facts.py` is the block at the end of this section. Save it outside the tree. It prints
`name`, `range`, `brings` and `build_time_code`, ready to paste. Then it prints a comment line of
native-code hints to read: packages with a `links` key, and packages with a `cc`, `cmake` or
`pkg-config` build dependency. Pass the range when the lockfile holds two ranges of the crate.

- **`brings` comes from the resolve, not from `cargo tree`.** It follows normal and build edges
  over all platforms, in the same resolve the record guard reads. `cargo tree` can print a
  narrower closure than that resolve, and with `-e normal,build` it prints nothing for a crate
  reached only through dev edges, such as tempfile. A record copied from it leaves locked
  packages that no record names.
- **`build_time_code` comes from a second resolve**, filtered to the two supported hosts,
  because spec 026 asks only for what builds there. The two lists can differ. serde_core's
  closure holds serde_derive through a `cfg(any())` dependency that never builds, so
  serde_derive is in its `brings` and not in its `build_time_code`.
- **`--all-features`** adds the features of workspace members that no default turns on. Today
  the members declare none, so it gives the same resolve as leaving it out.

The fuzz workspace, once it exists, is derived the same way with `--manifest-path
fuzz/Cargo.toml`.

```python
import json
import sys


def ranged(version):
    major, minor, patch = (int(part) for part in version.split("-")[0].split("+")[0].split("."))
    return str(major) if major else f"0.{minor}" if minor else f"0.0.{patch}"


def load(path):
    metadata = json.load(open(path))
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    return packages, nodes


def label(packages, package_id):
    package = packages[package_id]
    return f"{package['name']}@{ranged(package['version'])}"


def closure(nodes, root):
    seen, stack = set(), [root]
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            kinds = {kind["kind"] or "normal" for kind in dep["dep_kinds"]}
            if kinds & {"normal", "build"} and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    seen.discard(root)
    return seen


def find(packages, name, wanted):
    found = [i for i, p in packages.items() if p["source"] and p["name"] == name
             and (wanted is None or ranged(p["version"]) == wanted)]
    if len(found) != 1:
        sys.exit(f"{name}: {len(found)} locked third-party packages match; pass the range too")
    return found[0]


def quoted(items):
    return "[" + ", ".join(f'"{item}"' for item in items) + "]"


all_packages, all_nodes = load(sys.argv[1])
host_packages, host_nodes = load(sys.argv[2])
name, wanted = sys.argv[3], (sys.argv[4] if len(sys.argv) > 4 else None)
root = find(all_packages, name, wanted)
print(f'name = "{name}"')
print(f'range = "{ranged(all_packages[root]["version"])}"')
print("brings = " + quoted(sorted(label(all_packages, i) for i in closure(all_nodes, root))))
built, hints = [], []
on_hosts = closure(host_nodes, root) | {root} if root in host_nodes else set()
for i in sorted(on_hosts, key=lambda i: label(host_packages, i)):
    package = host_packages[i]
    kinds = {kind for target in package["targets"] for kind in target["kind"]}
    if "custom-build" in kinds:
        built.append(f"{label(host_packages, i)} build script")
    if "proc-macro" in kinds:
        built.append(f"{label(host_packages, i)} proc macro")
    if package.get("links"):
        hints.append(f"{label(host_packages, i)} links {package['links']}")
    builders = sorted(d["name"] for d in package["dependencies"] if d["kind"] == "build")
    if {"cc", "cmake", "pkg-config"} & set(builders):
        hints.append(f"{label(host_packages, i)} builds with {', '.join(builders)}")
print("build_time_code = " + quoted(built))
print("# native-code hints to read, not a field: " + ("; ".join(hints) or "none"))
```

## The fields written by hand

Read the crate at its locked version, in `~/.cargo/registry/src/*/<name>-<version>/`. Its
`.crate` file in `~/.cargo/registry/cache/*/` has the lockfile's `checksum` as its SHA-256, which
ties what you read to what is locked.

- **`reason`.** What the crate does for us, in a sentence or two.
- **`write_it_instead`.** Find the workspace's uses with `git grep -n '<crate>::'` and
  `git grep -n 'use <crate>'`, and for serde the derive attributes too. Estimate the lines it
  would take to write that much here, and say in `how` what was counted.
- **`exception`.** Empty, unless the owner approved one. An agent may record an approval it is
  carrying, and never originates one.
- **`expands_unsafe`.** Read every macro the crate exports or re-exports that our code calls or
  could call, derives included, and list those whose expansion holds `unsafe`. A macro our code
  cannot name, such as one in a package the crate does not re-export, is not listed.
- **`native_code`.** List the native code that the crate or a package in its `brings` compiles
  for a supported host, such as C, C++ or assembly files, and the native libraries they link
  there. Read every build script in `build_time_code` for what it compiles or links, and every
  `#[link]` attribute that applies on a supported host. Rust's own `asm!` is `unsafe` code, not
  native code.
- **`verdict`.** `kept-pending-owner` until the owner answers Q4.

## When a record changes

- The crate, or a package in its `brings`, moves to a new range. That is new code: derive the
  record again and read what changed.
- The closure changes, for example because a feature turns on another dependency.
- A PR that removes the last direct use of a crate deletes its record, so the record stops
  vouching for its `brings`.

A compatible bump that leaves the closure alone changes no record. The reviewer still reads the
diff between the two locked versions, as spec 026's "What the review checks" says.
