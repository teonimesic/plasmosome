---
id: 027
title: How attached software appears in a cell, and the test that the membrane stays invisible
status: draft
intents: [011, 004]
---

## Behavior

A plasmid's software is never installed into a running cell. It already exists outside the cell,
immutable and named by the digest of its contents. Attach makes it visible where a program already
looks for installed software: its files under `/opt/<plasmid>`, and its commands under
`/usr/local/bin`, which is first on the `PATH` every started process gets (spec 024). Detach stops
making it visible the way an uninstall does: no new lookup finds it, even from a directory inside
it, but a process that already opened, mapped or ran one of its files keeps that file until it
lets go. The kernel reports each holder it can find, and a detach never waits for one. The kernel
does all of this; the plasmid author writes none of it (spec 011).

Intent 011 asks for a membrane that the model and the harness never have to know about. This spec
says what it means for a workload to observe the membrane, and defines a differential test that
fails when it can. The same workload runs in a cell and in two reference lanes outside one. Where
the cell withholds something, it must give exactly the result an ordinary machine gives when that
thing does not exist. A workload that can tell "withheld" from "absent" has seen the membrane.

The layout, the detach accounting and the comparator can be built and shown able to fail now. The
cell lane waits for a booting cell ("Blocked on") and is reported as not run, never as passed.
Draft 028 is cited at `78d62c5` (#138); specs 022, 023 and 024 as accepted on main.

## Contract

### 1. What a plasmid's software is

- A **software tree** is a set of ordinary files from the plasmid's verified package: a spec 020
  import, or the files of a local plasmid. At attach the kernel records the SHA-256 of every file
  and the name of each file that is a command; the tree's digest covers both. A tree that cannot be
  pinned is refused before any effect. Nothing runs at attach, and nothing in a tree is writable by
  the workload.
- The declaration field that names the tree and its commands is **blocked on consumers**, under
  spec 011's rules. Spec 020 ships no permission bits and refuses symbolic links, so a command's
  execute bit can come only from that field. Until it exists no product tree has a command; tests
  declare commands through a fixture no product path reads. Whether the field may declare links
  inside a tree is the first consumer's to settle; a link never leaves its tree. A software command
  is a file on `PATH`, not the manifest's `[commands.<id>]` (a subject spawn).
- A software tree is not one of spec 017's five grant classes, because it opens no reach outside
  the cell. It is one effect of spec 008's attach transaction, with an exact inverse, so attach
  stays all-or-nothing and a restarted controller knows what is visible. Its journal record and
  attach wire are **RESERVED** until Q1 is answered, because their shape follows from the transport.
- No file in a tree is a managed inode, so spec 017's managed-file policy does not apply: a tree's
  bytes are immutable and kept while referenced (§4), and detach promises only the name.
- Complete observation accounts for every visible tree and name, through the field this spec adds
  to spec 001 ("Amendments").

### 2. The layout made at cell creation

Two guards hold whatever the layout: an attach never hides a file the root image has, and never
makes a name the image already resolves run different bytes.

- Before hello, PID1 makes `/opt` and `/usr/local/bin` in the workload root directories the kernel
  owns: empty at readiness, owned by root, mode 0755, with a mount source, type and options that
  name nothing about Plasmosome. They are slaves of PID1's side under spec 024 §5's propagation,
  and neither is shared. Propagation is fixed when the namespace is made and cannot be added later.
- The kernel adds and removes entries through handles it keeps, without entering the workload
  namespace. A running process sees a subtree created inside a filesystem it already has mounted,
  with no new mount (80 of 80, measured outside this repository in a privileged container,
  including on a virtio-fs share that libkrun did not serve). Item 2 re-runs it on the pinned guest.
- PID1 refuses readiness if the root image has any entry under either path, naming the path. This
  default has a cost: the official `node:22-bookworm` image has 8 entries in `/usr/local/bin` and
  `/opt/yarn-v1.22.22` (listed outside this repository), so it is refused. A layout that keeps the
  image's entries visible beneath the kernel's, such as a merge, meets the same guards (Q3).
- If owner gate O-7 adds a guest access-control layer, the order is mount, then confine, then exec.

### 3. Attach

1. Refusals come before any effect. A command name another attached plasmid already provides is
   code 102 with `target` the command's path; its message names the plasmid that holds it, and its
   `fix` names the field. A command name the root image already resolves on `PATH` is refused the
   same way. A file whose bytes do not match its recorded digest is code 108.
2. The bytes a name or a surviving reference reads are the bytes the journal's digests name. The
   workload cannot change them. A change on the host side is prevented under Q1(b) and detected
   under Q1(a).
3. The kernel places the tree at `/opt/<plasmid>`, and one entry per command in `/usr/local/bin`
   that resolves to the command's file in the tree.
4. Attach is not complete until the kernel has probed from inside the workload namespace: each
   command resolves through the `PATH` search a new process uses, and lands on the file identity
   (device and inode) recorded at placement. A call that returned success is not evidence: `mount`
   returned 0 while the workload's `stat` returned `ENOENT`, in four configurations, measured
   outside this repository in a privileged Linux container. Item 4 re-runs it.
5. No running process is signalled or restarted, and none has its environment, working directory or
   descriptors changed. A running shell finds a new command the next time its lookup misses.
6. A reload moves the names from the old tree to the new one, with no moment where a name resolves
   to nothing, and then detaches the old tree as §4 says.

### 4. Detach

- **Detach looks like an uninstall.** It removes every entry of the tree, as `rm -rf` does, and
  returns only after the kernel has probed from inside the workload namespace that no lookup of a
  name the plasmid placed succeeds. Each fails with `ENOENT`, as for software never installed; a
  shell that cached a command's path exits 127; and a working directory inside the tree lists
  nothing, with every lookup relative to it failing with `ENOENT`. So no new reference can be
  obtained once detach returns (spec 011).
- **Detach revokes the name, not the object.** An open descriptor, a mapping or a running
  executable keeps the bytes it had, and no holder is killed (spec 024 §10). Descriptors and
  mappings survived a lazy unmount 16 of 16 times, measured outside this repository in a booted
  libkrun guest; item 9 re-runs it on the pinned guest.
- **A detach completes when the names are gone.** That is the tree effect's observed absence for
  spec 008's finish. A surviving reference is reported, not residue: it never holds the journal
  pending, never sets `ready: false`, and no workload can veto or delay a detach with one.
- **Host bytes are kept while referenced**, so a reference never sees its file change or vanish.
  Under Q1(b) the guest removes the entries, and its kernel keeps open or mapped inodes alive. Under
  Q1(a) the host moves each file to a holding directory and removes the tree's directories. That
  directory is on the share's volume, because libkrun's macOS server reaches an inode through
  `/.vol/<dev>/<ino>`, which resolves only while the inode has a directory entry; inside the
  directory the helper's policy allows; and outside the exported share, so no guest path reaches
  it. Held files stay until the cell stops, since no guest signal proves the last reference gone.
- **The cache window under Q1(a).** libkrun v1.19.4 fixes the guest's entry and attribute cache
  timeouts at 5 s under `LINUX_COMPLETE`, and its API cannot change them, so under (a) a detach or
  reload probes and returns only once 5 s have passed since the host's last removal. Spec 017's
  zero timeouts protect revocable files and are not needed here: a file's bytes and attributes
  never change while it has a name, and a failed lookup is not cached, so an attach shows at once.
  A directory's cached attributes can lag by up to 5 s; the differential reports any probe that
  sees it.
- **References are found by identity, never by path:** the device and inode recorded at attach,
  and the mount ID where a tree is its own mount. The scan covers every mount namespace's table and
  every task's `fd`, `cwd`, `root`, `exe` and `maps` under `/proc/<pid>/task/<tid>/`, because a
  thread that called `unshare(CLONE_FS | CLONE_FILES)` (spec 024 §8 allows it) has a table only the
  task view shows (6 of 6, measured outside this repository in a container).
- **The scan's limit.** A descriptor in flight over `SCM_RIGHTS` is in no table, so no scan sees it
  (6 of 6, same container). Where a tree is its own filesystem, an `inotify` watch PID1 keeps on
  that filesystem's root, which the workload never sees, gets `IN_UNMOUNT` only when its superblock
  ends, once detach has removed both the workload's mount and PID1's own. Trees in one shared
  filesystem have no such signal, which is why Q1(a) keeps held files until the cell stops.
- **The report** has one entry `{path, pid, comm, exec_id?}` per reference the scan sees: the tree
  path it holds, the holding process and its command name, and the `exec_id` only when that process
  is the started process itself. A descendant, including one reparented to PID1, has none.
  Attribution is informational. The list is given once in the detach result, then in the cell's
  observation while the scan sees any reference, and stops when it sees none or the cell stops.
- **Failure restores nothing.** If a name cannot be removed, the detach stays committed without
  finish (spec 008 step 5): the desired state already lacks the plasmid, the cell is blocked for
  further mutation, and recovery resumes the removal. The plasmid's grants follow spec 017's
  removal; none is kept or restored.
- Detaching one plasmid leaves every other plasmid's names and trees, even with shared digests.

### 5. What "observing the membrane" means

A workload observes the membrane when anything it can see differs from what an ordinary machine
with the same image and the same reach shows it. There are five ways:

1. **A refusal that reveals policy:** a different errno, exit status or message; a hang where
   absence fails at once; `ECONNREFUSED` for a withheld host where an unknown host fails name
   resolution; an HTTP status or page from an intermediary.
2. **A hint**, a name or value the kernel wrote: in environment variables; in the names and
   arguments of any process the workload can see; in the hostname or resolver configuration; in
   mount sources, types and options; in the workload's cgroup path; in file names under `/`, `/run`,
   `/tmp`, `/opt` and `/usr/local`; in a kernel log; in `/proc/self/status`. An `exec_id` is one, so
   the workload never sees its own.
3. **An ordinary operation that behaves differently** on something the workload can reach: `mmap`,
   `exec`, links, `chmod`, `utimensat`, `rename`, locking or `fsync` on a workspace file, or running
   attached software.
4. **A denied kernel facility that fails differently** from the absent lane (§6), or with a signal.
5. **A disturbance at attach or detach:** a running process signalled or restarted, its environment,
   directory or descriptors changed, or an error a hand install or uninstall would not give.

Not closed here: timing and throughput (spec 006); resolved addresses, which are normalized, though
a program that inspects one can see spec 001 §4.2's `198.18.0.0/15`; and being a virtual machine,
since every lane is one, on the same kernel.

### 6. The differential

Three lanes run the same workload: the **cell lane**, a real cell whose attached plasmids grant a
set of reach G, and two **reference lanes**, test fixtures that no product path launches.

- **Both reference lanes** boot the same root image, kernel, command line, vCPUs, memory and
  devices with the same pinned libkrun; nothing on the host answers their vsock ports. An ordinary
  init replaces the trusted PID1 and starts the workload as spec 024 §4 does, except that only the
  absent lane loads its seccomp filter and sets §8's sysctls. It mounts `/opt`, `/usr/local/bin`
  and the workspace (an ordinary FUSE passthrough) as separate filesystems in one cgroup. Every
  name these carry (mount source, type, FUSE subtype, options, cgroup path) is a fixed neutral
  value from the manifest, never copied from the cell lane, and the cell lane must show the same.
  Fixture servers stand in for every host, and no lane reaches the network.
- **Open lane.** Everything exists: the hosts answer, the paths exist, the software is installed by
  hand, and with no 024 §8 mechanism every facility works. It is the positive control that shows
  each reach probe can see what it probes.
- **Absent lane.** The open lane with everything outside G removed: its names do not resolve, there
  is no default route, its paths are missing and its commands are not installed. For a kernel
  facility, "absent" is 024 §8's mechanism, which no lane could match without: unprivileged
  `unshare -U` succeeds under default sysctls (measured outside this repository in a container),
  and `socket(AF_VSOCK, ...)` needs no device (read from the kernel source, not measured). So §5.4
  tests what the cell adds beyond 024 §8, such as O-7's policy.
- When the cell lane attaches or detaches during a run, both reference lanes install or uninstall
  the same files by hand at the same moment, so G is the same in every lane at every step.

**Measurement rules.** A manifest fixed before the run declares every probe: its channel of §5, the
step it observes, and for a reach probe the kind of thing it reaches outside G (a host, a path or a
facility). Every channel has at least three probes, and every kind at least three reach probes.
Each lane runs twice from a fresh boot; a probe whose two runs differ after normalization is noise
and fails the run. Every probe writes a completion marker. A probe with no marker, or one that
failed before the step it observes, is `not run`, even when every lane fails it the same way, and
any `not run` makes the verdict `not run`. A reach probe equal in the open and absent lanes fails
the run.

**The verdict** passes when every probe ran in every lane, every reach probe discriminated, and the
cell lane equals the absent lane after normalization, on every probe and on the harness's tool
calls, results, exit statuses and files left behind. The only exceptions are §7's remaining
differences, each allowed mechanically: the manifest names the probes and fields where it shows
and the exact value the cell lane may have there, such as mode 0600 for a file the workload
created. Any other difference, in those probes or the harness transcript, fails the verdict.

**The workload** is the probe suite plus one unmodified harness pinned by version and digest (Q3),
attached in the cell lane and installed by hand elsewhere, driven by a recorded model session from
a fixture endpoint.

**Normalization** is a fixed table of `(probe, field, rule)` in the manifest; any other field is
compared exactly. A rule replaces each value with a placeholder numbered by first appearance within
that probe and field in that lane's run, so equal values stay equal, distinct ones stay distinct,
time placeholders keep their order, and an extra value in one probe shifts no other. The rules cover
process IDs, boot IDs, mount and parent mount IDs, device and inode numbers, times and durations the
system chose, ephemeral ports, resolved addresses, and memory and load counters. A value the
workload set, such as a time set with `utimensat`, is never normalized, and nothing is normalized
for how it looks. A new rule is an edit to this spec, with the reason.

**Results** go in a dated file under `docs/benchmarks/`: each lane's configuration; the digests of
the image, kernel, libkrun, fixtures, harness, session, probe suite and manifest; the normalization
table as run; and every probe's status and observation in every run of every lane.

### 7. Differences from an ordinary machine

The absent lane shows these too, so the differential does not judge them, but the workload can see
each one:

- spec 024 §4's start: `NoNewPrivs: 1`, `CapBnd: 0000000000000000` and `Seccomp: 2`; exactly
  `PATH`, `HOME` and `LANG`, with no `USER`, `SHELL` or `LOGNAME`; standard input on `/dev/null`
  and no terminal; `RLIMIT_CORE` 0 and an `RLIMIT_NOFILE` hard limit of at most 4,096; a umask with
  no group or other write; a session of its own and no supplementary groups;
- spec 024 §8's errnos: `EAFNOSUPPORT` for `AF_VSOCK`, `ENOSPC` for a user namespace, `EPERM` for
  `io_uring_setup`;
- spec 024 §5's `/dev`, the missing `/sys`, and the device list in `/proc/interrupts`;
- three separate filesystems: their types (a FUSE workspace, and virtio-fs under Q1(a)), their
  `master:N` tags, and `EXDEV` on a rename between them.

Remaining, so the differential reports them:

- Spec 017's managed-file policy: no file-backed mapping or direct execution of a workspace file.
- Spec 001 §4.2's data verbs: no symbolic or hard links, FIFOs or Unix sockets in the workspace
  (git's fsmonitor socket is one), no time or mode set, and files created 0600. So `ln -s`,
  `chmod +x`, `cp -p`, `tar x` and a checkout with links behave differently.
- Spec 001 §4.2's brokers at `/.plasmosome/brokers/`, a path that names the membrane.
- Spec 001 §4.2's TUN device, synthetic prefix and `127.0.0.53` resolver, in the interface list
  and the resolver configuration.
- Spec 024 §5's missing PID namespace: PID1 and its threads, kernel thread names, and PID1's mount
  table through the world-readable `/proc/1/mountinfo` (measured outside this repository in a
  container).
- Spec 024 §9: a Mount withdrawn, fenced (draft 028) or refused under Force, under a running
  process, fails each held reference with 024 §9's error (expected `ENOTCONN`). A fenced Mount with
  no peer also stays in the mount table until its removal finishes, while a fresh lookup there
  fails with `ENOENT`. The reference lanes stage the nearest ordinary analogue, aborting their FUSE
  connection and unmounting at the same moment, and the differential reports what still differs.

Naming these does not waive them. Q2 asks the owner about the six remaining ones; until then, item
15 allows exactly these, at the probes and fields the manifest names, and nothing else.

## Amendments this spec proposes

These are proposed text. The PR that accepts this spec edits no other accepted document; each
amendment is applied by a later reviewed change to the document it amends, before any task that
relies on it.

- **Spec 011, detach (`011:195-207`, `011:359-363`, `011:434-437`).** Draft spec 028 carries the
  one text for these lines. It covers software-tree references as §4 does, with the `SCM_RIGHTS`
  limit, and composes spec 024's insertion. This spec proposes no text of its own there.
- **Spec 011, out of scope (`011:489-493`).** Replace "and its spec belongs to the isolation work
  under intent 011." with "and spec 027 specifies it."
- **Spec 001 §3.11 and §3.12.** Add `"held": []` to both example results, and after §3.11's:
  "`held` lists each reference into the plasmid's software, or a reload's replaced tree, that the
  guest's scan saw once its names were gone, as `{path, pid, comm, exec_id}` (spec 027). It is
  not residue, so `residue` stays `"empty"`."
- **Spec 001 §4.2, observation.** In `GuestObservation`, after `network:GuestNetworkObservation`,
  add `software:SoftwareObservation`. After "cannot stand in for another.", add:
  > SoftwareObservation is the strict record `{trees:[SoftwareTree], held:[TreeReference]}` (spec
  > 027). SoftwareTree names one tree visible in the workload namespace, with its digest and every
  > name placed for it; its field names follow spec 027's journal record. TreeReference is
  > `{path:String, pid:u64, comm:String, exec_id:String|null}`, one per reference into a detached
  > tree that the guest's scan sees. Both arrays are required even when empty. A tree or name with
  > no committed or pending journal effect, or a committed effect with no tree or name, fails
  > observation. A `held` row is not a binding or residue, and never makes a removal incomplete.
- **Spec 001 §4.2, PID1.** After spec 024's proposed replacement text, which ends "with `/workload`
  moved over `/` in those namespaces.", add "Before hello it also makes the software directories of
  spec 027."
- **Spec 024.** §5's image contract, after "a writable `/tmp` with mode 1777", gains ", and, under
  spec 027's default layout, nothing under `/opt` or `/usr/local/bin`". §6, after "creates one
  cgroup v2 group for the workload.", gains "Its path names nothing about Plasmosome (spec 027)."
  §10, after "only what that plasmid made visible.", gains "A started process holding a file of a
  detached software tree keeps it (spec 027 §4)." Out of scope: "(`011:489-493`)" becomes ", which
  spec 027 specifies".
- **Spec 020 §6.** After "Imported files are read-only and non-executable to the workload", add
  "except files a spec 027 software tree declares as commands, which are executable and read-only".
- **Only if the owner chooses Q1(a).** In spec 001 §4.2, replace "Do not export a host directory
  through `krun_set_root` or virtiofs." with "Do not export a host directory through
  `krun_set_root` or virtiofs, except one read-only software share per cell (spec 027), which
  holds only digest-checked trees and nothing revocable or secret.", and after "the two declared
  bridge endpoints" add "and read-only access to that cell's software directory (spec 027)". In
  spec 017, after "No host descriptor or uncontrolled host-directory export is passed to the
  workload.", add "Spec 027's software share is a controlled export: it holds immutable trees,
  never a managed inode." In spec 023 §9, replace "with no virtiofs export" with "with no virtiofs
  export but spec 027's read-only software share".
- **Only if the owner chooses Q1(b).** In spec 011's out-of-scope bullet, replace "A plasmid's
  software is not copied into a running cell:" with "A plasmid's software is not installed into a
  running cell, though its bytes may be copied there unchanged:".

## Requests to draft specs

- **028 §4.** Name the still-mounted fenced target of §7 as the one way a fence differs from a
  withdrawal.

## Open questions

These need the owner. Each has the default this spec works to until answered.

1. **Q1, where the bytes come from.** Default: (a), gated on item 17; (b) if any gate fails.
   - **(a) A read-only virtio-fs share per cell**, empty at creation. Attach places the tree in it
     as a copy or copy-on-write clone, never a hard link into the store. It keeps live attach and
     spec 011's "not copied into a running cell". Its gate, measured by the first task on the
     pinned libkrun v1.19.4 and guest kernel, not in a container:
     - the workload sees the share owned by root, which needs `KRUN_SEMANTICS_LINUX_COMPLETE` or an
       equivalent (under `LINUX_SIMPLIFIED` the workload would see itself as the owner);
     - mounted without `ro`, a workload write fails with `EACCES`, as on any root-owned 0755
       directory, and a guest-root write reaches the server and fails with `EROFS`;
     - a held file stays readable after the host moves it to the holding directory (§4);
     - after the cache window, a directory held inside a detached tree lists nothing and every
       lookup in it fails with `ENOENT`;
     - neither the share nor its holding directory contains a Mount source or lies inside one,
       spec 022 §3 rule 5's test applied both ways.

     Guards: nothing revocable or secret is placed in it; its tag and source are the manifest's
     neutral values. A same-UID host process can still write the per-cell copy, so a change is
     detected, not prevented: the supervisor's own observation re-hashes a placed file whenever its
     size, inode or change time differs from the one recorded at placement, which a writer cannot
     set back without root, and a mismatch fails observation, naming the file. Costs: each detach
     and reload keeps the old tree on the host until the cell stops, and waits out §4's 5 s. It
     changes specs 001, 017 and 023 §9 ("Amendments"), and the helper's policy must give read-only
     access to exactly the cell's software directory, which needs per-cell parameters (O-2): the
     qualified policy allows writes anywhere under the work directory, and `read_only` is enforced
     inside the process the guest drives.
   - **(b) A copy over the data channel** into storage only PID1 can reach, checked against the
     digests, then exposed. Spec 001 stays as it is, and spec 017 already lets an authorized private
     copy be mapped and executed. It costs guest memory or disk and attach time, and spec 011's
     wording changes ("Amendments"). A copy kept as its own filesystem gives each tree a superblock,
     and with it §4's liveness signal, and its detach is an ordinary removal in the guest.
   - A boot-time read-only disk is rejected: libkrun adds disks only before the VM starts.
2. **Q2, §7's remaining differences.** Change specs 017 and 001 so the workspace behaves like a
   directory, amend spec 024 for a PID namespace with an ordinary-looking PID1, or record each as a
   known exception to intent 011? Mounting `/proc` with `hidepid` would only move the difference:
   uid 1000 would see no `/proc/1` while an orphan's `getppid()` still returns 1. The owner also
   accepts §7's first list, which the workload can see too, by accepting spec 024. Default: item 15
   allows exactly the remaining set, and intent 011 is recorded as partly served.
3. **Q3, which harness and image.** Default: Claude Code's native build, the harness the owner uses
   today, run as `claude -p` because spec 024 gives no terminal, pinned by version and digest and
   attached as one tree; on a root image built from `debian:bookworm-slim` with spec 024's
   `workload` user, pinned by digest. That base has nothing under `/opt` or `/usr/local/bin`
   (listed outside this repository in a container), so it passes §2's guard. The owner may change
   either, accept the merged layout so Node images qualify, or require a second runtime to pass.

## Acceptance

Each item names the broken implementation it catches, and the level its evidence can reach:
**mechanism** (any Linux kernel, in a VM or a privileged container), **outside** (the pinned libkrun
and guest kernel with no cell, as the reference lanes run) or **cell** (the pinned guest in a real
cell, not run until one boots).

1. **Layout, cell.** At readiness `/opt` and `/usr/local/bin` are empty, root-owned, mode 0755,
   slaves and not shared, with the manifest's neutral source, type and options. An image with an
   entry under either path fails readiness naming it. Catches: `shared` propagation, which leaks
   workload mounts back, and hidden image files.
2. **No restart, cell.** A process started before attach runs the command by name in a loop. It
   exits 0 only if a run exited 127 before one succeeded, and its environment, directory and
   descriptors at the end equal those at its start. Catches: a per-process view taken at start, a
   restart, and an edited `PATH`.
3. **Refusals, cell.** A second plasmid that provides an attached command name is 102, naming the
   holder, and leaves no trace; so is a name the image resolves on `PATH`. Catches: shadowing.
4. **Probe from inside, mechanism.** A placement whose call succeeds but which the workload
   namespace cannot see fails the attach, staged through `private` propagation and through a bind
   from a source the namespace cannot reach. The same staging made visible succeeds. Catches:
   trusting a return value.
5. **Bytes, cell.** A file whose bytes differ from its digest is 108, and no name appears. After
   attach, a workload write fails, and writes to the source and to the store leave what the name
   reads unchanged. Under Q1(a), a host write to the per-cell copy, including one that restores the
   file's modification time, fails the supervisor's next observation, naming the file. Catches:
   checking once and then trusting the path, and a re-hash keyed on modification time.
6. **Reload, cell.** A test hook stops the reload after each step. At every stop the command
   resolves, and once the reload returns it runs the new bytes. Catches: remove-then-add.
7. **All or nothing, cell.** A fault at the last command's placement leaves none of the plasmid's
   names and no tree, and the desired state after the abort equals the one before. Catches: a
   rollback that skips names placed earlier.
8. **Names gone, mechanism and cell.** Right after detach returns, a new process running the command
   exits 127 and `stat /opt/<plasmid>` is `ENOENT`, as in the absent lane, and a running shell that
   cached the path gets the message the open lane's hand uninstall gives. A listing of every
   directory the workload can reach under `/opt`, and its mount table, shows no detached tree and no
   holding directory. Catches: a name left behind, including one a cache still serves, and a
   holding directory inside the share.
9. **References, mechanism and cell.** One detach under five holders: the started process with a
   descriptor, its child running the detached binary, an orphan reparented to PID1 with its
   directory in the tree, a thread with its own table after `unshare(CLONE_FS | CLONE_FILES)`, and
   a descriptor in flight over `SCM_RIGHTS`. Detach returns without waiting; its report names the
   first four, with an `exec_id` on the started process only. Then the descriptors read the
   original bytes, the binary runs, the orphan's directory lists nothing and lookups in it fail
   with `ENOENT`, and the in-flight descriptor, received once the report is empty, reads the
   original bytes. A detach with no holder reports none. Catches: a detach that waits or can be
   vetoed, a per-process scan, a descendant given an `exec_id`, a held directory that still
   resolves files, bytes deleted when the report empties, and invented references.
10. **Report lifetime, cell.** A holder that closes before detach returns is never reported. One
    that keeps its reference is in every observation until it closes, and in none after. While it
    holds, the journal has finished, `ready` is true and residue lists nothing. A detach whose name
    removal is made to fail stays committed without finish, blocks the next mutation, restores no
    name or grant, and its leftover name does not fail observation. Catches: a report that never
    stops or stops early, a tree reference counted as residue, a failure that restores, and a
    pending removal that recovery cannot observe.
11. **Exactness, cell.** Trees A and B share every file digest and differ in command names. Detach
    A: B's names resolve to unchanged bytes, and A's are gone. Catches: sharing by digest that
    revokes both.
12. **Measurement rules and results, outside.** A staged suite has a reach probe that crashes, one
    whose setup fails the same way in every lane, one that never discriminates, one that reads a
    random value, and a channel one probe short. The first two make the verdict `not run`, the
    others fail the run. The results hold every field §6 names, enough for a stranger to rerun
    every lane. Catches: dropped probes, noise counted as reach, and results that cannot tell no
    probes from no differences.
13. **The comparator can fail, outside.** Each mutant is the absent lane with one planted change,
    and the comparator fails each on exactly the probe aimed at it, with every other probe equal:
    an environment variable naming the membrane; an `exec_id` in the environment; a process, mount
    or cgroup name containing `plasmosome`; an `/opt` mount of another type, with a source naming
    the membrane, or `ro` (`EROFS` where the absent lane gives `EACCES`); a withheld host answered
    with `ECONNREFUSED` or a hang; a denied call answered with `SIGSYS`; an extra `PATH` directory;
    a `/proc/self/status` line that differs. With §7's allowances in force, a new difference in a
    probe they name, such as a second entry beside `/.plasmosome` or a second resolver line, still
    fails. Catches: a comparator that cannot fail, one that excuses a whole probe, and a mutant
    that fails only because its lane crashed.
14. **Normalization keeps relations, outside.** Three mutants must fail: a 64-hex token as an
    environment value, a workspace that ignores `utimensat`, and two distinct workspace files that
    report one inode. A hard-linked pair still compares equal, an extra process in one lane changes
    no other probe's placeholders, and a table with a rule §6 does not list fails before comparing.
    Catches: normalizing by look, losing equality or order, and placeholders shifted across probes.
15. **The differential, cell.** The verdict of §6 passes, each of §7's remaining differences
    allowed only where the manifest names it. A cell whose `/opt` source names the membrane fails.
    Catches: a suite that avoids the known differences, one that lets a new one through, and names
    copied from the cell into the reference lanes.
16. **Observation, cell.** A tree or name in the software directories with no committed or pending
    journal effect makes complete observation fail, naming it, and so does a committed effect whose
    tree or name is missing. Catches: an inventory built from the journal alone, or from the
    directories alone.
17. **The Q1(a) gate, outside.** Each of Q1(a)'s five measurements passes, and fails against its
    mutants: a share served with `LINUX_SIMPLIFIED`; one mounted `ro`, which gives the workload
    `EROFS`; one added with `read_only` false, where the guest-root write succeeds; a host that
    deletes a detached file instead of moving it; a host that leaves the tree's directories in
    place; and a controller that accepts a Mount whose source contains the share or lies inside
    it. Catches: adopting (a) on container evidence.

## Blocked on

- **A booting cell that runs spec 024's `exec`,** for every cell item. Three owner gates stop it:
  - O-1: the product check refuses an instance root under the admin-writable `/usr/local/var`;
  - O-6: the qualified libkrun carries a leftover library search path to remove before re-signing;
  - O-7: the guest policy mechanism, since the qualified kernel has no security modules. Hello
    needs it.
- **Q1,** for §1's journal record and attach wire, and every cell item but 1. Under (a) it also
  needs O-2: per-cell parameters in the helper's host policy.
- **This spec's amendments to spec 001,** for the detach result of items 9 and 10 and the
  observation field of items 10 and 16.
- **Consumers,** for §1's declaration field in product plasmids. Tests use the fixture of §1.
