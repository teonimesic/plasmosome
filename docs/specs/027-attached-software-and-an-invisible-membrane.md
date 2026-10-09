---
id: 027
title: How attached software appears in a cell, and the test that the membrane stays invisible
status: draft
intents: [011, 004, 012]
---

## Behavior

A plasmid's software is never installed into a running cell. It already exists outside the cell,
immutable and named by the digest of its contents. Attach makes it visible in the places a program
already looks for installed software: its files under `/opt/<plasmid>`, and its commands under
`/usr/local/bin`, which is first on the `PATH` every started process gets (spec 024). Detach stops
making it visible. After a detach no new lookup finds it, but a process that already opened or
mapped one of its files keeps that file until it lets go, and the kernel names each such holder
until the last one is gone. The layout that gives a later attach somewhere to land is made when the
cell is created, because it cannot be made in a cell that is already running. The kernel does all
of this. The plasmid author writes none of it (spec 011).

Intent 011 asks for a membrane that the model and the harness never have to know about. Today no
test checks the word "invisible". This spec says what it means for a workload to observe the
membrane, and defines a differential test that fails when it can. The same workload runs in a cell
and outside one. The two runs may differ only where the cell withholds something. Where they
differ, the cell must give exactly the result an ordinary machine gives when that thing does not
exist. A workload that can tell "withheld" from "absent" has seen the membrane, and the test fails.

**What can be built now, and what waits.** The layout and the detach accounting can be tested now
on any Linux kernel, and built into spec 024's guest PID1 once it exists. How the bytes reach the
guest is an owner question (Q1). The differential's comparator and its two outside lanes can be
built and shown able to fail now. Its cell lane needs a cell that boots and runs spec 024's
`exec`, and none does yet: owner gates O-1, O-6 and O-7 block a real boot. Until then the cell
lane is reported as not run. It is never reported as passed. The host is Darwin arm64 and the
guest is Linux under libkrun. Each acceptance item names the level its evidence can reach.

**Why spec 001 does not list intent 011.** Spec 009 derives which work counts toward an intent from
the intents of the specs a task names. Spec 001 governs the wire protocol, the status daemon and
recovery messages, and most of its delivered tasks have nothing to do with invisibility. Listing
011 there would credit all of them to it, and spec 009's check would then report an honest
`served: none` for intent 011 as contradicted by delivered work. So spec 001 keeps
`[003, 004, 009, 012]`. This spec carries 011, and so does draft spec 024. A runtime task whose
deliverable makes the membrane invisible names spec 027 or spec 024 among its specs.

## Contract

### 1. What a plasmid's software is

- A **software tree** is a set of ordinary files from the plasmid's verified package: a spec 020
  import, or the files of a local plasmid. At attach the kernel records the SHA-256 of every file,
  which files are executable, and which command names map to which files. A local source is pinned
  this way too, so the journal knows exactly which bytes were visible. A tree that cannot be pinned
  is refused before any effect.
- Nothing runs at attach. There is no install hook, no build step and no post-processing.
- Nothing in the tree is writable by the workload.
- The declaration field that names the tree and its commands is **blocked on consumers**. No
  plasmid ships workload software yet. The first one that does decides the field's shape, under
  spec 011's rules: the field is described in plain words, and every refusal names the field and
  carries a `fix`.
- A software tree is not one of spec 017's five grant classes, because it opens no reach outside
  the cell. Running its bytes reaches only what the cell's grants allow anyway. It is still one
  effect of spec 008's attach transaction, with an exact inverse, so attach stays all-or-nothing
  and a restarted controller knows what is visible. The journal record and the wire verbs that
  carry it are **RESERVED** until Q1 is answered, because their shape follows from the transport.
- Spec 017 forbids mapping or executing *revocable* host-backed files without another accepted
  mechanism. A software tree is that other mechanism, for a different kind of file: its bytes are
  immutable, a holder may keep them after detach, and detach promises only the name. So no file in
  a tree is a managed inode, and spec 017's managed-file policy does not apply to it.
- The guest's complete observation (spec 001 §4.2) accounts for every visible tree and every name
  the kernel placed. A tree or name the kernel cannot attribute is an observation fault, never an
  empty list.

### 2. The layout made at cell creation

When PID1 creates the workload mount namespace (spec 024 §4), and before its hello, it also makes
the two software directories. These rules apply to both:

- `/opt` and `/usr/local/bin` in the workload root are each a filesystem the kernel owns. Each is
  empty at readiness, owned by root and mode 0755. Its mount source, type and options name nothing
  about Plasmosome.
- The workload side is a **slave** of PID1's side. A mount PID1 makes appears to the workload, and
  nothing the workload does propagates back. `shared` is refused, because it is a two-way door:
  measured in a privileged Linux container, the cell pushed its own mounts back into the kernel's
  table in every sample. `private` is refused, because it swallows a later attach without an error.
  The flag that decides this is the one on the parent when the namespace is created, so it cannot
  be fixed in a running cell. That is why it is set here.
- The kernel keeps a handle on each directory. It adds and removes entries through that handle,
  without entering the workload namespace. A subtree created inside a filesystem that a running
  process already has mounted needs no mount at all. Measured in the same container, such a subtree
  was readable and executable by an already-running process in 80 of 80 samples, including when
  the filesystem was a host-served virtio-fs share.
- PID1 refuses readiness if the root image has any entry at either path, and names the path.
  Mounting over them would hide the image's files without saying so.
- If owner decision O-7 adds a guest access-control layer such as Landlock, the order is mount,
  then confine, then exec. A grant names the mountpoint, never an overlay's layers. Measured in a
  booted libkrun guest: a confined thread could not mount (`EPERM`, 15 of 15). A mount made under a
  granted directory after the domain existed was covered (15 of 15). A grant on a layer did not
  reach a merge built from it, so a wrong grant fails closed with `EACCES` (15 of 15). That guest
  ran a different kernel from the qualified one, which is built without security modules.

### 3. Attach

1. Refusals come before any effect. A command name another attached plasmid already provides is
   code 102 with `target` the command's path. Its message names the plasmid that holds it, and its
   `fix` names the field. A command name that the root image already resolves on `PATH` is refused
   the same way. Attach widens the cell; making an existing name run different bytes is not
   widening. A file whose bytes do not match its recorded digest is code 108.
2. The bytes reach the guest by the transport Q1 chooses. Whatever it is, the bytes a name exposes
   are the bytes the journal's digests name, for as long as that name exists. No transport may let
   the host side or the workload change those bytes after they were checked.
3. The kernel places the tree at `/opt/<plasmid>` and one entry per command at
   `/usr/local/bin/<command>`. Each entry is an ordinary symbolic link to the file in the tree, the
   way software installed by hand often looks.
4. Attach is not complete until the kernel has probed from inside the workload namespace. Each
   command resolves through the same `PATH` search a newly started process uses, and lands on the
   file identity recorded at attach (device and inode). A placement call that returned success is
   not evidence. Measured in the container, `mount` returned 0 with nothing on stderr while the
   cell's `stat` returned `ENOENT`, in four separate configurations.
5. No running process is signalled or restarted, and none has its environment, working directory
   or descriptors changed. A process started after attach finds the commands by its `PATH` search.
   A running shell finds a new command the next time its lookup misses.
6. A reload moves the names from the old tree to the new one. There is no moment when a name
   resolves to nothing.
7. Attach never edits any process's environment. Removing a `PATH` entry from a running process was
   measured to work only when the process reassigned its own `PATH`. Removing one from outside the
   process is untested, and this design does not depend on it.

### 4. Detach

- Detach returns only after the kernel has probed from inside the workload namespace that no
  new lookup of any name the plasmid placed succeeds. Each lookup fails with `ENOENT`, as it would
  for software that was never there. A shell that had cached a command's path gets exit status 127,
  as it would after an ordinary uninstall.
- **Detach revokes the name, not the object.** An open descriptor, a mapping, a working directory
  or an executable image inside the tree keeps working. Measured in a booted libkrun guest, 16 of
  16: with references held, a plain unmount refused with `EBUSY`; a lazy unmount succeeded while
  the descriptor stayed readable and the mapping kept executing; a descriptor alone, or a mapping
  alone, kept the filesystem alive. The kernel does not kill a holder: removing a plasmid does not
  stop a started process (spec 024 §7).
- **The kernel finds references by identity, never by path.** It looks for the device and inode, or
  the mount ID, recorded at attach. It checks every process's descriptors, mappings, working
  directory, root and executable, and every mount namespace. An observer keyed on names goes quiet
  while the object survives: measured in the guest, observers keyed on names reported zero objects
  while the filesystem was alive, readable and still holding its space.
- A clean unmount proves nothing. `EBUSY` is only available at the mount the references were taken
  through, and only before a lazy unmount. Unmounting the source after the target's lazy unmount
  succeeded in 16 of 16 samples with both references still live.
- **Each surviving reference is reported with its owner.** The owner is the `exec_id` of the started
  process that holds it (spec 024). That is spec 011's detach contract made concrete for software:
  - A reference with no owner fails the detach. Examples are a reference PID1 holds, or a copy of
    the mount left in another namespace. This check runs before detach returns, where it can still
    fail it.
  - A reference still held when the plasmid's drain budget runs out is reported against the owner
    named at detach. It becomes visible after detach has returned, so it is an obligation the kernel
    keeps afterwards, not a detach failure.
- Detaching one plasmid leaves every other plasmid's names and trees. Two plasmids whose trees have
  the same digest still have separate names. Detaching one leaves the other's names resolving.

### 5. What "observing the membrane" means

A workload observes the membrane when anything it can see differs from what an ordinary machine
with the same image and the same reach shows it. There are five ways this happens:

1. **A refusal that reveals policy.** Something withheld fails differently from something absent,
   so the error vocabulary changes: a different errno, exit status or message text; a hang where
   absence fails at once; `ECONNREFUSED` for a withheld host where an unknown host fails name
   resolution; an HTTP status or page from an intermediary.
2. **A hint in the environment.** A name or value the kernel wrote that an ordinary machine would
   not have. It can sit in environment variables; the names and arguments of any process the
   workload can see; the hostname or resolver configuration; mount sources, types and options; file
   names under `/`, `/run`, `/tmp`, `/opt` and `/usr/local`; a readable kernel log; or status lines
   such as `Seccomp:` in `/proc/self/status`.
3. **An ordinary operation that behaves differently on something the workload can reach.** For
   example `mmap`, `exec`, symbolic and hard links, `chmod`, `rename`, locking or `fsync` on a
   workspace file; or running attached software.
4. **A denied kernel facility that fails differently.** For example `AF_VSOCK`, user namespaces,
   `mount`, `bpf` or tracing PID1. Each must fail with the errno the same call gets in the absent
   lane of section 6. It must never fail with a signal.
5. **A disturbance at attach or detach.** A running process is signalled or restarted, or its
   environment, directory or descriptors change.

Some things this spec does not close, and it names them so silence is not read as a claim:

- Timing and throughput. Spec 006 measures performance.
- Resolved addresses. These are normalized below. A program that inspects them can still see the
  `198.18.0.0/15` synthetic range of spec 001 §4.2.
- Being a virtual machine. Every lane is one, on the same kernel.

### 6. The differential

Three lanes run the same workload. The first is a real cell. The other two are test fixtures that
no product path launches.

- **Cell lane.** A real cell. The attached plasmids grant a set of reach, G.
- **Open lane.** The same root image bytes, guest kernel, kernel command line, vCPU count and
  memory, booted by the same pinned libkrun. An ordinary init at the same path replaces the
  trusted PID1. The network and filesystem are ordinary. The workload starts exactly as spec 024
  §3 starts it: same user, argv, environment, directory and descriptors. Everything G names
  exists here ordinarily: fixture servers for its hosts, the workspace as a plain directory, and
  the software installed by hand at the same `/opt` and `/usr/local/bin` paths. Everything outside
  G that the cell lane's host also has exists here too, and is reachable.
- **Absent lane.** The open lane with everything outside G removed. Unknown names do not resolve,
  there is no default route and no vsock device, the paths are missing and the commands are not
  installed.

The verdict:

- **Pass.** Every probe's observation in the cell lane equals the absent lane's, after the
  normalization below.
- **Not vacuous.** Every probe counted as reaching outside G differs between the open and absent
  lanes. A probe that does not differ never touched the boundary, and it is not counted.
- **Able to fail.** The comparator fails each planted mutant in the acceptance.

Spec 024 §3's start conditions are the same in every lane by construction, so this test does not
judge them. Whether they suit a harness stays spec 024's open question 5.

**The workload** has two parts:

- A probe suite covering each of the five ways in section 5.
- One unmodified harness, pinned by version and digest. In the cell lane it arrives as attached
  software. It is driven by a recorded model session, served from a fixture endpoint reachable in
  every lane. The comparison covers every tool call it makes, each result, its exit status, and the
  files it leaves in the workspace.

**Normalization** replaces only these values: process IDs and `exec_id`s; times, dates and
durations; boot IDs and random bytes; device and inode numbers; resolved addresses and ephemeral
ports. Everything else is compared exactly. Adding to this list is an edit to this spec, with the
reason. A test run that normalizes anything else fails.

**Structural differences** are allowed in one place only. The cell lane has mounts at `/opt`,
`/usr/local/bin` and the workspace target that the absent lane does not. They may show as separate
filesystems, in the mount table and to `statfs`, provided their source, type and options carry no
hint. Nothing else is allowed to differ.

**Results** go in a dated file under `docs/benchmarks/`. It records each lane's configuration and
the digests of the image, kernel, libkrun, harness, recorded session and probe suite. It records
the normalization list as run, and every difference found, with its probe and both observations.

### 7. Differences accepted specs already cause

The differential will report these today. They come from accepted contracts, and this spec does not
decide them (Q2). Naming them here does not waive them: the cell-lane acceptance stays unmet while
any of them stands.

- Spec 017's managed-file policy forbids any file-backed mapping of a workspace file, and any direct
  execution of one. Programs that map files, and binaries a build writes into the workspace, fail in
  the cell and work in the absent lane.
- Spec 001 §4.2's data verbs create no symbolic or hard links, change no mode, and create files
  as 0600. So `ln -s`, `chmod +x`, and a checkout containing symbolic links fail in the cell and
  work in the absent lane.
- Spec 001 §4.2 exposes brokers at `/.plasmosome/brokers/`, a path in the workload's root that
  names the membrane.

## Changes proposed to other specs

These are proposed text. They are made when this spec is accepted, not in this PR. The one edit
this PR makes to another spec is the last bullet of spec 011's "Out of scope", which now points
here.

- **Spec 024 §4.** Add: before hello, PID1 also mounts the kernel-owned `/opt` and `/usr/local/bin`
  of spec 027 §2, with the workload side a slave. It refuses readiness, naming the path, when the
  root image has an entry at either one.
- **Spec 024 §5.** Replace "Each attempt returns an ordinary error, such as `EPERM`, `EACCES` or
  `EAFNOSUPPORT`" with: "Each attempt returns the errno the same kernel gives the same unprivileged
  user when the facility is absent, as spec 027's absent lane observes it."
- **Spec 024 acceptance 4.** Each call must return the absent lane's errno, not just any error.
- **Spec 024 §7.** Add: a detach takes away the names, and a started process that holds a file of a
  detached tree is reported as its owner (spec 027 §4).
- **Spec 020 §2 and §6.** "Imported files are read-only and non-executable to the workload" gains
  an exception: files a software tree marks executable are executable, and still read-only.
- **Spec 001 §4.2.** Mention the software directories where PID1's namespaces are described. If Q1
  chooses the virtio-fs route, replace "Do not export a host directory through `krun_set_root` or
  virtiofs" with the one read-only per-cell export of Q1(a) and its guards.

## Open questions

These need the owner. This spec does not decide them.

1. **Q1, where the bytes come from.**
   - **(a) A read-only virtio-fs share per cell**, added at cell creation and empty at first. Attach
     places the tree inside it, so the guest sees a new subtree in a filesystem it already has
     mounted. This is the measured route, and the only one that keeps spec 011's "not copied into a
     running cell" literally true. It needs spec 001 §4.2's virtio-fs ban relaxed for this one
     export. Its guards: tree entries are copies or copy-on-write clones, never hard links into the
     shared store, so a change to one copy reaches neither the store nor another cell; the host side
     is read-only; the share's tag names nothing; and the helper's host policy admits only that
     directory. This is the recommendation.
   - **(b) A copy over the existing data channel** into storage only PID1 can reach, checked
     against the digests, then exposed. Spec 001 stays as it is, and spec 017 already lets a checked
     private copy execute. It costs guest memory or disk and attach time, and spec 011's wording
     would need to allow the copy.
   - A boot-time read-only disk is rejected: libkrun's API adds disks only before the VM starts, so
     a later attach would have nowhere to land.
2. **Q2, the differences of section 7.** Change spec 017's and spec 001's mechanisms so that the
   workspace behaves like a directory, or record each one as a known exception to intent 011 in its
   outcome? The first is a large redesign of managed files. The second makes intent 011 partly
   served on purpose.
3. **Q3, which harness.** Intent 011 names "the harnesses I use today". Which one is pinned first,
   and should a second one with a different runtime (for example one built on Node and one native
   binary) be required before the test counts as passing?

## Acceptance

Each item names the broken implementation it catches. **Level** says where its evidence can come
from:

- **mechanism:** any Linux kernel, in a VM or a privileged container. Generic VFS behaviour carries
  to the guest.
- **outside:** the open and absent lanes, with no cell.
- **cell:** the pinned guest in a real cell. Gated on a booting cell; reported as not run until
  then.

1. **Layout, mechanism and cell.** Two namespaces made the same way, except that one workload side
   is `slave` and the other `private`. A mount on PID1's side appears in the first and not the
   second. A third, made `shared`, lets a workload-side mount appear on PID1's side; under `slave`
   it does not. In a cell, the workload's own mount table shows both software directories as
   slaves and neither as shared. Catches: `private`, which swallows attaches, and `shared`, which
   leaks mounts back, including in PID1's real layout rather than a test double.
2. **No restart, cell.** A process started before attach runs the attached command by name in a
   loop. It exits 0 only if at least one run exited 127 before one succeeded, and if the
   environment, working directory and descriptors it recorded at its start equal those at its end.
   Its `exec_id` was issued before the attach. Catches: a per-process view taken at start, a
   restart (a restarted copy never sees the 127), and an edited `PATH`.
3. **Refusals, cell.** Attaching a second plasmid that provides an attached command name is 102,
   naming the holder, and leaves no trace. So is a command name the image already resolves on
   `PATH`. An image with an entry in `/opt`, and another with one in `/usr/local/bin`, each fail
   readiness, naming the path. Catches: silent shadowing and overwrite.
4. **Probe from inside, mechanism.** A placement whose call succeeds but which the workload
   namespace cannot see fails the attach. The test stages that through `private` propagation, and
   through a bind from a source unreachable in the target namespace. The same staging made visible
   succeeds. Catches: trusting a syscall's return value.
5. **Bytes, cell.** A file whose bytes differ from its digest is 108, and no name appears. After
   attach, a write from the workload and a change on the host side each leave what the name reads
   unchanged. Catches: checking once and then trusting the path.
6. **Reload, cell.** A test hook stops the reload after each of its steps. At every stop the
   command resolves, and after the last one it runs the new bytes. Catches: remove-then-add, which
   a timing loop could miss.
7. **All or nothing, cell.** A fault injected at the last command's placement leaves none of the
   plasmid's names and no tree, and the journal holds no effect for it. The fault is placed last on
   purpose, so it lands after other names were already placed. Catches: a rollback that skips
   names placed earlier.
8. **Names gone, mechanism and cell.** Right after detach returns, a new process running the
   command exits 127 and `stat /opt/<plasmid>` is `ENOENT`. Both match the absent lane, where the
   software was never installed. Catches: a name left behind, including one a cache still serves.
9. **Owner or failure, mechanism and cell.** Two stagings that differ only in the owner:
   - In the first, one started process holds a descriptor into the tree and another is running the
     detached binary from its mapping. Detach succeeds. It reports both references with their
     `exec_id`s, the descriptor still reads, and the binary still runs.
   - In the second, the reference is held by something that is not a started process: a
     descriptor a helper outside any `exec` holds, or a copy of the mount left in another
     namespace. Detach fails and names that object.

   Catches: a failure branch no case reaches, and an observer keyed on names, which reports nothing
   once the name is gone.
10. **Bound, cell.** A holder keeps its mapping past the drain budget. After detach has returned
    success, a report names that reference against the `exec_id` named at detach. No case requires
    the detach itself to have failed. Catches: claiming a detach-time check of a condition that
    arises later.
11. **Clean unmount is not absence, mechanism.** After the target's lazy unmount, unmounting the
    source succeeds while references live, and the detach report still names them. Catches:
    treating a successful unmount as proof nothing remains.
12. **Exactness, cell.** Plasmids A and B carry trees with the same digest. Detach A: B's commands
    resolve and A's do not. Catches: sharing by digest that revokes both.
13. **The probes reach the boundary, outside.** Every probe counted as reaching outside G gives a
    different observation in the open and absent lanes. Catches: probes that never cross the
    boundary, which would pass against any membrane.
14. **The comparator can fail, outside.** Each mutant is the absent lane with one planted change.
    The comparator fails each one and names the probe and both observations:
    - an environment variable naming the membrane;
    - a process or mount whose name contains `plasmosome`;
    - a withheld host answered with `ECONNREFUSED`, or with a hang, instead of failed resolution;
    - a denied syscall answered with `SIGSYS`;
    - `/opt` mounted read-only, which returns `EROFS` where the absent lane returns `EACCES`;
    - an extra `PATH` directory;
    - a `Seccomp:` line that differs.

    Catches: a comparator that cannot fail.
15. **Normalization is fixed, outside.** A run whose normalization list differs from section 6's
    fails before comparing. Catches: normalizing a difference away.
16. **The differential, cell.** The cell lane equals the absent lane on every probe and on the
    harness transcript, after normalization, with only the structural differences of section 6.
    The suite has at least one probe for each difference of section 7. Until those are resolved,
    this item is expected to fail on exactly those probes, and the results file lists them. A run
    with no cell is reported as not run. Catches: a suite that avoids the known differences.
17. **Observation, cell.** A tree or command entry placed in the software directories with no
    journal effect behind it makes complete observation fail, naming it. Catches: an inventory
    built from the journal instead of from what is there.
18. **Results, outside and cell.** The results file holds every field section 6 lists. A stranger
    can rerun every lane from it alone.

## Blocked on

- **A booting cell that runs spec 024's `exec`.** It blocks items 2, 3, 5 to 7, 10, 12, 16 and 17,
  and the cell half of items 1, 8, 9 and 18. Owner gates O-1 and O-6 block a real boot. O-7 blocks
  a readiness hello, and with it every `exec`.
- **Q1.** It blocks the journal record and the wire verbs of section 1, and items 5 to 7 and 17.
- **Consumers.** The declaration field of section 1 waits for the first plasmid that ships workload
  software.
