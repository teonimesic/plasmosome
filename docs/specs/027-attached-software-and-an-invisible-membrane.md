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
`/usr/local/bin`, which is first on the `PATH` every started process gets (draft spec 024). Detach
stops making it visible. After a detach no new lookup finds it, but a process that already opened
or mapped one of its files keeps that file until it lets go, and the kernel reports each holder it
can find. A detach never waits for a holder. The kernel does all of this; the plasmid author writes
none of it (spec 011).

Intent 011 asks for a membrane that the model and the harness never have to know about. This spec
says what it means for a workload to observe the membrane, and defines a differential test that
fails when it can. The same workload runs in a cell and in two reference lanes outside one. Where
the cell withholds something, it must give exactly the result an ordinary machine gives when that
thing does not exist. A workload that can tell "withheld" from "absent" has seen the membrane.

The layout, the detach accounting and the comparator can be built and shown able to fail now. The
cell lane waits for a booting cell ("Blocked on") and is reported as not run, never as passed.
Drafts are cited at their heads: 024 at `51ad798` (#127), 022 at `6faf75d`, 023 at `f165662`.

## Contract

### 1. What a plasmid's software is

- A **software tree** is a set of ordinary files from the plasmid's verified package: a spec 020
  import, or the files of a local plasmid. At attach the kernel records the SHA-256 of every file
  and the name of each file that is a command; the tree's digest covers both. A tree that cannot be
  pinned is refused before any effect. Nothing runs at attach, and nothing in a tree is writable by
  the workload.
- The declaration field that names the tree and its commands is **blocked on consumers**, under
  spec 011's rules. Spec 020 ships no permission bits and refuses symbolic links, so a command's
  execute bit can come only from that field; until it exists no tree has a command. Whether it may
  declare links inside a tree is the first consumer's to settle; a link never leaves its tree. A
  software command is a file on `PATH`, not the manifest's `[commands.<id>]` (a subject spawn).
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
  name nothing about Plasmosome. They are slaves of PID1's side under draft 024 §5's propagation,
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
2. Whatever transport Q1 picks, the bytes a name or a surviving reference reads are the bytes the
   journal's digests name, and neither the host side nor the workload can change them later.
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
   to nothing.

### 4. Detach

- Detach returns only after the kernel has probed from inside the workload namespace that no new
  lookup of any name the plasmid placed succeeds. Each fails with `ENOENT`, as for software that
  was never there, and a shell that cached a command's path exits 127.
- **Detach revokes the name, not the object.** An open descriptor, a mapping, a working directory
  or a running executable in the tree keeps working, and the kernel kills no holder (draft 024
  §10). This held 16 of 16 times outside this repository, in a booted libkrun guest with each tree
  its own mount; item 9 re-runs it on the pinned guest.
- **A detach completes when the names are gone.** That is the tree effect's observed absence for
  spec 008's finish. A surviving reference is reported, not residue: it never holds the journal
  pending, never sets `ready: false`, and no workload can veto or delay a detach with one.
- **Host bytes are kept while referenced**, so a reference never sees its file change or vanish.
  Under Q1(a) the host moves a detached tree aside and keeps it until the cell stops: libkrun's
  macOS server reaches an inode through `/.vol/<dev>/<ino>`, which resolves only while the inode
  has a directory entry, and no guest signal proves the last reference gone. Under Q1(b) the copy
  is in the guest, where unlinking keeps every open or mapped inode alive.
- **References are found by identity, never by path:** the device and inode recorded at attach,
  and the mount ID where a tree is its own mount. The scan covers every mount namespace's table and
  every task's `fd`, `cwd`, `root`, `exe` and `maps` under `/proc/<pid>/task/<tid>/`, because a
  thread that called `unshare(CLONE_FS | CLONE_FILES)` (draft 024 §8 allows it) has a table only
  the task view shows (6 of 6, measured outside this repository in a container).
- **The scan's limit.** A descriptor in flight over `SCM_RIGHTS` is in no table, so no scan sees it
  (6 of 6, same container). Where a tree is its own mount, its superblock's end is a signal that
  needs no enumeration: an `inotify` watch on its root gets `IN_UNMOUNT` only then. Where trees
  share one filesystem there is no such signal, which is why Q1(a) keeps bytes until the cell stops.
- **The report** has one entry `{path, pid, comm, exec_id?}` per reference found: the tree path it
  holds, the holding process and its command name, and the `exec_id` only when that process is the
  started process itself. A descendant, including one reparented to PID1, has none. Attribution is
  informational. The list is given once in the detach result, then in the cell's observation while
  the scan finds any reference, and stops when it finds none or the cell stops.
- **Failure restores nothing.** If a name cannot be removed, the detach's generation stays
  committed without finish (spec 008, publication step 5): the desired state already lacks the
  plasmid, the cell is blocked for further mutation, and recovery resumes the removal. The
  plasmid's grants follow spec 017's removal; none is kept or restored.
- Detaching one plasmid leaves every other plasmid's names and trees, even with shared digests.

### 5. What "observing the membrane" means

A workload observes the membrane when anything it can see differs from what an ordinary machine
with the same image and the same reach shows it. There are five ways:

1. **A refusal that reveals policy:** a different errno, exit status or message; a hang where
   absence fails at once; `ECONNREFUSED` for a withheld host where an unknown host fails name
   resolution; an HTTP status or page from an intermediary.
2. **A hint**, a name or value the kernel wrote: in environment variables; in the names and
   arguments of any process the workload can see; in the hostname or resolver configuration; in
   mount sources, types and options; in file names under `/`, `/run`, `/tmp`, `/opt` and
   `/usr/local`; in a kernel log; in `/proc/self/status`. An `exec_id` is one, so the workload never
   sees its own.
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
  init replaces the trusted PID1 and starts the workload exactly as draft 024 §4 does, seccomp and
  capability sets included. It sets 024 §8's sysctls, gives 024 §5's `/proc` and `/dev`, and
  mounts `/opt`, `/usr/local/bin` and the workspace as separate filesystems with the cell lane's
  types, sources, options and propagation, the workspace through an ordinary FUSE passthrough.
  Fixture servers stand in for every host, and no lane reaches the network.
- **Open lane.** Everything G names and everything outside G exists: the fixture hosts answer, the
  paths exist, and the software is installed by hand.
- **Absent lane.** Everything outside G is missing: its names do not resolve, there is no default
  route, its paths are missing and its commands are not installed. Within G it equals the open lane.
- When the cell lane attaches or detaches during a run, both reference lanes install or uninstall
  the same files by hand at the same moment, so G is the same in every lane at every step.
- **"Absent" for a kernel facility** is the copied mechanism: `user.max_user_namespaces=0` for user
  namespaces, the seccomp allowlist for `AF_VSOCK`. Without them no lane could match: unprivileged
  `unshare -U` succeeds under default sysctls (measured outside this repository in a container),
  and `socket(AF_VSOCK, ...)` needs no device. So §5.4 tests what the cell adds beyond 024 §8,
  such as O-7's policy. What is copied is still a difference from a stock machine (§7).

**Measurement rules.** A manifest fixed before the run declares every probe: its channel of §5, the
step it observes, and for a reach probe the kind of thing it reaches outside G (a host, a path or a
facility). Every channel has at least three probes, and every kind at least three reach probes.
Each lane runs twice from a fresh boot; a probe whose two runs differ is noise and fails the run.
Every probe writes a completion marker. A probe with no marker, or one that failed before the step
it observes, is `not run`, even when every lane fails it the same way, and any `not run` makes the
verdict `not run`. A reach probe that is equal in the open and absent lanes fails the run.

**The verdict** passes when every probe ran in every lane, every reach probe discriminated, and
every cell-lane observation equals the absent lane's after normalization, except exactly §7's
remaining differences. The comparator must fail every planted mutant (item 13).

**The workload** is the probe suite plus one unmodified harness pinned by version and digest,
attached in the cell lane and installed by hand elsewhere, driven by a recorded model session from
a fixture endpoint. Every tool call, result, exit status and file it leaves is compared.

**Normalization** is a fixed table of `(probe, field, rule)` in the manifest; any other field is
compared exactly. A rule replaces each value with a placeholder numbered by first appearance in
that lane's run, so equal values stay equal and distinct ones distinct, and time placeholders keep
their order. The rules cover process IDs, boot IDs, mount and parent mount IDs, device and inode
numbers, times and durations the system chose, ephemeral ports, resolved addresses, and memory and
load counters. A value the workload set, such as a time set with `utimensat`, is never normalized,
and nothing is normalized for how it looks. A new rule is an edit to this spec, with the reason.

**Results** go in a dated file under `docs/benchmarks/`: each lane's configuration; the digests of
the image, kernel, libkrun, fixtures, harness, session, probe suite and manifest; the normalization
table as run; and every probe's status and observation in every run of every lane.

### 7. Differences from an ordinary machine

Copied into the reference lanes, so the differential does not judge them:

- draft 024 §4's start state: `NoNewPrivs: 1`, `CapBnd: 0000000000000000` and `Seccomp: 2` in
  `/proc/self/status`;
- draft 024 §8's errnos: `EAFNOSUPPORT` for `AF_VSOCK`, `ENOSPC` for a user namespace, `EPERM` for
  `io_uring_setup`;
- draft 024 §5's `/dev`, the missing `/sys`, and the device list in `/proc/interrupts`;
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
- Draft 024 §5's missing PID namespace: PID1's name, helpers and threads, kernel thread names, and
  PID1's mount table through the world-readable `/proc/1/mountinfo` (measured outside this
  repository in a container).
- Draft 024 §9: a Mount withdrawn under a running process, or fenced by draft 028, fails every held
  reference with `ENOTCONN`. The reference lanes stage the nearest ordinary analogue, aborting
  their FUSE connection at the same moment, and the differential reports what still differs.

Naming these does not waive them. Q2 asks the owner about the six remaining ones; until then, item
15 allows exactly these and nothing else.

## Amendments this spec proposes

Proposed text for accepted specs, applied by the PR that accepts this spec. This PR edits only spec
011's last "Out of scope" bullet, to "draft spec 027 proposes it"; acceptance drops "draft".

- **Spec 001 §4.2, observation.** In `GuestObservation`, after `network:GuestNetworkObservation`,
  add `software:SoftwareObservation`. After "cannot stand in for another.", add:
  > SoftwareObservation is the strict record `{trees:[SoftwareTree], held:[TreeReference]}` (spec
  > 027). SoftwareTree is `{plasmid:String, digest:String, names:[String]}`, one per tree visible
  > in the workload namespace. TreeReference is `{path:String, pid:u64, comm:String,
  > exec_id:String|null}`, one per reference into a detached tree that the guest's scan finds. Both
  > arrays are required even when empty. A tree or name with no journal effect, or an effect with no
  > tree or name, fails observation. A `held` row is not a binding or residue, and never makes a
  > removal incomplete.
- **Spec 020 §6.** After "Imported files are read-only and non-executable to the workload", add
  "except files a spec 027 software tree declares as commands, which are executable and read-only".
- **Only if the owner chooses Q1(a).** In spec 001 §4.2, replace "Do not export a host directory
  through `krun_set_root` or virtiofs." with "Do not export a host directory through
  `krun_set_root` or virtiofs, except one read-only software share per cell (spec 027), which
  holds only digest-checked trees and nothing revocable or secret.", and after "the two declared
  bridge endpoints" add "and read-only access to that cell's software share". In spec 017, after
  "No host descriptor or uncontrolled host-directory export is passed to the workload.", add "Spec
  027's software share is a controlled export: it holds immutable trees, never a managed inode."

## Requests to draft specs

- **024 §5.** Make `/opt` and `/usr/local/bin` before hello as §2 says; under the default layout,
  add "nothing under either path" to the image contract; and end the proposed spec 001 §4.2 PID1
  text with "Before hello it also makes the software directories of spec 027." Consider mounting
  the workload's `/proc` with `hidepid=invisible`, which hides PID1, its helpers and kernel threads
  from uid 1000 (§7); 024 item 13's census would then need a privileged test hook.
- **024 §10.** Add: "A started process holding a file of a detached tree keeps it (spec 027 §4)."
- **028.** Cite §4's report shape, and count no tree reference as residue.

## Open questions

These need the owner. Each has the default this spec works to until answered.

1. **Q1, where the bytes come from.** Default: (a), gated on item 17; (b) if any gate fails.
   - **(a) A read-only virtio-fs share per cell**, empty at creation. Attach places the tree in it
     as a copy or copy-on-write clone, never a hard link into the store. It keeps live attach and
     spec 011's "not copied into a running cell". The first task measures its gate on the pinned
     libkrun v1.19.4 guest, not in a container:
     - the workload sees the share owned by root, which needs `KRUN_SEMANTICS_LINUX_COMPLETE` or an
       equivalent (under `LINUX_SIMPLIFIED` the workload would see itself as the owner);
     - a workload write fails with `EACCES`, as on any root-owned 0755 directory, and a guest-root
       write reaches the server and fails with `EROFS`; the share is mounted without `ro`;
     - a detached tree's host bytes stay readable through a surviving reference (§4);
     - the share lies outside every Mount source, as draft 022 §3 rule 5 requires of the operator
       file, so no read-write Mount reaches its bytes.

     Guards: nothing revocable or secret is placed in it; its tag and source name nothing; its
     entry, attribute and negative-entry cache timeouts are zero, as spec 017 requires of managed
     targets (libkrun's default is 5 s). It changes spec 001's ban and allowlist and spec 017's
     export sentence ("Amendments"). The helper's host policy must then give read-only access to
     exactly this directory, which needs per-cell parameters (O-2): the qualified policy allows
     writes anywhere under the work directory, and `read_only` is enforced inside the process the
     guest drives. Draft 023 §9's argument then rests on that policy alone.
   - **(b) A copy over the data channel** into storage only PID1 can reach, checked against the
     digests, then exposed. Spec 001 stays as it is, and spec 017 already lets an authorized private
     copy be mapped and executed. It costs guest memory or disk and attach time, and spec 011's
     wording would need to allow the copy. A copy kept as its own read-only filesystem gives each
     tree a superblock, and with it §4's liveness signal.
   - A boot-time read-only disk is rejected: libkrun adds disks only before the VM starts.
2. **Q2, §7's remaining differences.** Change specs 017 and 001 so the workspace behaves like a
   directory, ask draft 024 for a PID namespace or `hidepid`, or record each as a known exception
   to intent 011? Default: item 15 allows exactly that set, and 011 is recorded as partly served.
3. **Q3, which harness and image.** Default: one non-interactive harness the owner uses today
   (draft 024 has no terminal), on an image that passes §2's guard, which excludes the official
   Node images unless the owner accepts the merged layout. Must a second runtime pass too?

## Acceptance

Each item names the broken implementation it catches, and the level its evidence can reach:
**mechanism** (any Linux kernel, in a VM or a privileged container), **outside** (the reference
lanes, no cell) or **cell** (the pinned guest in a real cell, not run until one boots).

1. **Layout, cell.** At readiness `/opt` and `/usr/local/bin` are empty, root-owned, mode 0755, and
   slaves, not shared, in the workload's mount table. An image with an entry under either path
   fails readiness naming it. Catches: `shared` propagation, which leaks workload mounts back, and
   hidden image files.
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
   reads unchanged. Under Q1(a), a write to the per-cell copy fails the host's observation of the
   share. Catches: checking once and then trusting the path.
6. **Reload, cell.** A test hook stops the reload after each step. At every stop the command
   resolves, and after the last it runs the new bytes. Catches: remove-then-add.
7. **All or nothing, cell.** A fault at the last command's placement leaves none of the plasmid's
   names and no tree, and the desired state after the abort equals the one before. Catches: a
   rollback that skips names placed earlier.
8. **Names gone, mechanism and cell.** Right after detach returns, a new process running the command
   exits 127 and `stat /opt/<plasmid>` is `ENOENT`, as in the absent lane, and a running shell that
   cached the path gets the message the open lane's hand uninstall gives. Catches: a name left
   behind, including one a cache still serves.
9. **References, mechanism and cell.** One detach under five holders: the started process with a
   descriptor, its child running the detached binary, an orphan reparented to PID1 with its
   directory in the tree, a thread with its own table after `unshare(CLONE_FS | CLONE_FILES)`, and
   a descriptor in flight over `SCM_RIGHTS`. Detach returns without waiting. The report names the
   first four, with an `exec_id` on the started process only; every holder still works; and the
   in-flight descriptor, received once the report is empty, reads the original bytes. A detach with
   no holder reports none. Catches: a detach that waits or can be vetoed, a per-process scan, a
   descendant given an `exec_id`, bytes deleted when the report empties, and invented references.
10. **Report lifetime, cell.** A holder that closes before detach returns is never reported. One
    that keeps its reference is in every observation until it closes, and in none after. While it
    holds, the journal has finished, `ready` is true and residue lists nothing. A detach whose name
    removal is made to fail stays committed without finish, blocks the next mutation, and restores
    no name or grant. Catches: a report that never stops or stops early, a tree reference counted
    as residue, and a failure that restores.
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
    an environment variable naming the membrane; an `exec_id` in the environment; a process or
    mount name containing `plasmosome`; an `/opt` mount of another type, with a source naming the
    membrane, or `ro` (`EROFS` where the absent lane gives `EACCES`); a withheld host answered with
    `ECONNREFUSED` or a hang; a denied call answered with `SIGSYS`; an extra `PATH` directory; a
    `/proc/self/status` line that differs. Catches: a comparator that cannot fail, and a mutant
    that fails only because its lane crashed.
14. **Normalization keeps relations, outside.** Three mutants must fail: a 64-hex token as an
    environment value, a workspace that ignores `utimensat`, and two distinct workspace files that
    report one inode. A hard-linked pair still compares equal, and a table with a rule §6 does not
    list fails before comparing. Catches: normalizing by look, and losing equality or order.
15. **The differential, cell.** The cell and absent lanes are equal on every probe and the harness
    transcript, after normalization, except exactly §7's remaining differences, each covered by a
    probe. Catches: a suite that avoids the known differences, and one that lets a new one through.
16. **Observation, cell.** A tree or name in the software directories with no journal effect makes
    complete observation fail, naming it, and so does a journal effect whose tree or name is
    missing. Catches: an inventory built from the journal alone, or from the directories alone.
17. **The Q1(a) gate, cell.** Each of Q1(a)'s four measurements passes on the pinned guest and
    fails against its mutant: a share served with `LINUX_SIMPLIFIED`; one mounted `ro`, which gives
    the workload `EROFS`; a host that deletes a detached tree; and a controller that accepts a Mount
    whose source contains the share. Catches: adopting (a) on container evidence.

## Blocked on

- **A booting cell that runs draft 024's `exec`,** for every cell item. Three owner gates stop it:
  - O-1: the product check refuses an instance root under the admin-writable `/usr/local/var`;
  - O-6: the qualified libkrun carries a leftover library search path to remove before re-signing;
  - O-7: the guest policy mechanism, since the qualified kernel has no security modules. Hello
    needs it.
- **Q1,** for §1's journal record and attach wire, and every cell item but 1 and 17, which place no
  tree. Under (a) it also needs O-2: per-cell parameters in the helper's host policy.
- **This spec's amendment to spec 001,** for item 16's observation field.
- **Consumers,** for §1's declaration field.
