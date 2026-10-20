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
- `brings` and `build_time_code` are arrays copied from the derivation. They are sorted by the
  whole string, in code-point order, so `zerocopy-derive@0.8` comes before `zerocopy@0.8`.
- `expands_unsafe` and `native_code` are the string `"none"`, or a non-empty array of strings.
  Each string reads `<name>@<range> <what>`, and names the crate or a package in its `brings`.
- `exception` is the empty string unless the owner approved an exception. Then it holds the
  argument and a link to where the approval was given.
- `verdict` is `admitted`, `kept-pending-owner` or `replace`.
- Features, and which of our crates use a dependency, are not kept in the record. Read them from
  `cargo tree`.

Long strings may use TOML's multi-line form, `"""\`, with a backslash ending each line. A `#`
comment may explain a derived list; comments are not fields.

## Deriving `range`, `brings` and `build_time_code`

Derive at the commit whose Cargo.lock the record describes. For a PR that changes Cargo.lock,
that is the PR's head, not the commit it branches from. On a contributor's commit, run cargo only
after the steps in the "Read before running" paragraph of
[spec 026, section 1](../specs/026-security-standard.md#1-dependency-admission). `cargo metadata`
runs no build script or proc macro, and `--locked` makes it fail rather than re-resolve, so the
facts are the lockfile's. Its output holds absolute paths from your machine, so the commands
write it to a temporary directory outside the tree:

```shell
dir=$(mktemp -d)
cargo metadata --locked --all-features --format-version 1 > "$dir/all.json"
cargo metadata --locked --all-features --format-version 1 \
  --filter-platform aarch64-apple-darwin --filter-platform x86_64-unknown-linux-gnu \
  > "$dir/hosts.json"
python3 "$dir/record-facts.py" "$dir/all.json" "$dir/hosts.json" <crate> [<range>]
```

Save the block at the end of this section as `$dir/record-facts.py`. Pass the range when the
lockfile holds two ranges of the crate. The block prints `name`, `range`, `brings` and
`build_time_code`, ready to paste. Then it prints two comment lines, which are not fields:

- **What leaves Cargo.lock if the workspace stops using the crate.** The block removes the
  workspace members' direct edges to the crate, and walks every edge from the members, dev edges
  included. It lists the third-party packages it no longer reaches. Each package it lists does
  leave. More can leave, when dropping the crate also turns a feature off.
- **Native-code hints to read.** These are packages with a `links` key, packages with a `cc`,
  `cmake` or `pkg-config` build dependency, and every library that a `link(name = "...")`
  attribute names in a package's sources. The hints ignore `cfg`, so they do not say which of
  those libraries a supported host links.

How the lists are made:

- **`brings` comes from the resolve, not from `cargo tree`.** It is the closure over normal and
  build edges, not dev edges, on all platforms, in the same resolve the record guard reads.
  `cargo tree` can print a narrower closure than that resolve, and with `-e normal,build` it
  prints nothing for a crate reached only through dev edges, such as tempfile. A record copied
  from it leaves locked packages that no record names. Build edges matter: proptest's `brings`
  begins with `autocfg@1`, which it reaches only through num-traits' build edge.
- **`build_time_code` comes from a second resolve**, filtered to the two supported hosts,
  because spec 026 asks only for what builds there. The two lists can differ. serde_core's
  closure holds serde_derive through a `cfg(any())` dependency that never builds, so
  serde_derive is in its `brings` and not in its `build_time_code`.
- **`--all-features`** adds the features of workspace members that no default turns on. Today
  the members declare none, so it gives the same resolve as leaving it out.

**The fuzz workspace.** Today the records describe the root Cargo.lock only. Once
`fuzz/Cargo.lock` exists, derive each record again with `--manifest-path fuzz/Cargo.toml` on both
commands. Its `brings` and `build_time_code` are then the union of the two derivations. If the two
lockfiles lock different ranges of one crate, the PR that adds the fuzz workspace decides how the
record shows that.

```python
import json
import pathlib
import re
import sys

LINK = re.compile(r'link\(\s*name\s*=\s*"([^"]+)"')


def ranged(version):
    major, minor, patch = (int(part) for part in version.split("-")[0].split("+")[0].split("."))
    return str(major) if major else f"0.{minor}" if minor else f"0.0.{patch}"


def load(path):
    metadata = json.load(open(path))
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    return packages, nodes, metadata["workspace_members"]


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


def kept_without(nodes, members, root):
    seen, stack = set(members), list(members)
    while stack:
        here = stack.pop()
        for dep in nodes[here]["deps"]:
            if (here in members and dep["pkg"] == root) or dep["pkg"] in seen:
                continue
            seen.add(dep["pkg"])
            stack.append(dep["pkg"])
    return seen


def linked(package):
    names = set()
    for path in pathlib.Path(package["manifest_path"]).parent.rglob("*.rs"):
        names |= set(LINK.findall(path.read_text(errors="replace")))
    return sorted(names)


def find(packages, name, wanted):
    found = [i for i, p in packages.items() if p["source"] and p["name"] == name
             and (wanted is None or ranged(p["version"]) == wanted)]
    if len(found) != 1:
        sys.exit(f"{name}: {len(found)} locked third-party packages match; pass the range too")
    return found[0]


def quoted(items):
    return "[" + ", ".join(f'"{item}"' for item in items) + "]"


all_packages, all_nodes, members = load(sys.argv[1])
host_packages, host_nodes, _ = load(sys.argv[2])
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
    libraries = linked(package)
    if libraries:
        hints.append(f"{label(host_packages, i)} names in link(name): {', '.join(libraries)}")
print("build_time_code = " + quoted(built))
kept = kept_without(all_nodes, set(members), root)
gone = sorted(label(all_packages, i) for i, p in all_packages.items()
              if p["source"] and i not in kept)
print(f"# leaves Cargo.lock with it: {len(gone)}: " + (", ".join(gone) or "none, it stays"))
print("# native-code hints to read, not a field: " + ("; ".join(hints) or "none"))
```

## The fields written by hand

Read the crate at its locked version, in `~/.cargo/registry/src/*/<name>-<version>/`. Its
`.crate` file in `~/.cargo/registry/cache/*/` has the lockfile's `checksum` as its SHA-256, which
ties what you read to what is locked.

- **`reason`.** What the crate does for us, in a sentence or two. Say what it provides, not which
  of our crates use it, because the record does not keep that.
- **`write_it_instead`.** Every record uses one scale: the lines of non-test Rust that a
  replacement would need for the parts this workspace uses. Tests and test vectors are not
  counted. Uses in our own tests are, because the replacement must serve them too. A replacement
  may use the standard library and the other direct dependencies, and `how` says when it does.
  `how` holds four things:
  - Each use count, with the exact command that produced it. Search with the pathspec
    `-- '*.rs'`, which also keeps these records from counting themselves: for example,
    `git grep -l serde_json -- '*.rs' | wc -l`. Search for `<crate>::` and for `use <crate>`,
    and for a derive, its attributes. Then read the code, because a method called on an imported
    type matches neither search. Do not pass `-w` to `grep -o`: the BSD grep that macOS ships
    then drops matches.
  - The date and the commit the counts were measured at, written first, as "On 2026-10-20, at
    main 8fe24e3,". Both are required. The date is the day the commands ran, and the commit is
    the one whose Rust files they searched. Together they mark the counts as a snapshot. Later
    commits move them, so a re-run on a newer commit that gives another number shows drift, not
    a wrong record.
  - The parts a replacement needs, each with its lines in parentheses, such as `(150 lines)` or
    `(150)`, and then their sum, as `: 700 lines`. `lines` holds that sum.
  - Whether replacing the crate would remove anything from Cargo.lock, from the block's
    "leaves Cargo.lock" line.
- **`exception`.** Empty, unless the owner approved one. An agent may record an approval it is
  carrying, and never originates one.
- **`expands_unsafe`.** Read every macro the crate exports or re-exports that our code calls or
  could call, derives included, and list those whose expansion holds `unsafe`. A macro our code
  cannot name, such as one in a package the crate does not re-export, is not listed.
- **`native_code`.** List the native code that the crate or a package in its `brings` compiles
  for a supported host, such as C, C++ or assembly files, and the native libraries they link
  there. Read every build script in `build_time_code` for what it compiles or links. Read each
  `link(name)` the hints line names, to see whether it applies on a supported host. Rust's own
  `asm!` is `unsafe` code, not native code.
- **`verdict`.** `kept-pending-owner` until the owner answers Q4.

## When a record changes

Run the block again, at the head of the PR, whenever that PR's Cargo.lock changes the version of
the crate or of any package in its `brings`, or the packages they depend on. Update every derived
field to what the block prints. A version can change a record without changing its range or its
closure: quote 1.0.40 has no build script and quote 1.0.45 has one, so a move between them
changes `build_time_code` in every record whose `brings` holds `quote@1`.

- Read the diff between the two locked versions of each package that moved, as spec 026's "What
  the review checks" says. Update `native_code` and `expands_unsafe` when the diff adds or removes
  native code, or a macro whose expansion holds `unsafe`.
- A package that moves to a new range is new code: read it as well as the diff.
- A PR that removes the last direct use of a crate deletes its record, so the record stops
  vouching for its `brings`.

A PR whose Cargo.lock changes no version in the crate's closure, and no package in it, leaves the
record's derived fields as they are.
