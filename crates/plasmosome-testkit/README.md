# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, the
scenarios that cross crate boundaries, and the membrane differential's probe manifest and
comparator.

Three things live here. The first is the **conformance suite** — the behavioral clauses of the
`EnforcementBackend` contract, written once as functions generic over the trait. Every backend
is held to the same functions, unchanged; a backend that fails one is the thing that is wrong.
That is what makes the fake a model of enforcement rather than a hope about it.

`FakeBackend` and `CompositeBackend` over three fake leaves both pass all thirteen clauses. The
composite failed three of the original clauses when it was first wired in, because it lost the
handle its leaf issued; task 008 fixed the backend rather than the clauses, which is the point of
holding every backend to the same functions.

Every clause reads the backend's complete account, and a snapshot that fails is a contract
failure. Each account check compares the whole snapshot with what the clause expects: the standing
objects, an issued record for each standing grant and none for an applied or planted object, and
no unfinished effect. Two defects hold the clauses to that. `SnapshotOmitsIssuedRecords` reports
its objects with no records, and `snapshot_never_invents_objects` must reject it.
`SnapshotReconstructsGrantsFromObjects` rebuilds a record for every standing object, and
`apply_and_removal_reach_the_universe` must reject it. One per-step check is narrower: after each
revoke, `planted_residue_survives_unrelated_revoke` checks only that the planted residue still
stands, and that clause ends with a check of the whole snapshot.

The exact-grant clauses build expected objects from each prepared operation, never from a
backend's returned entry, and the returned handle must be the address the caller prepared.
Applied-object withdrawal keeps observations from every other universe class standing. Dedicated
defective backends alter each colliding resource field, substitute an owner, remove cross-class
state, and choose the wrong exact instance. Three change only a recipe or launch, in the
returned entry, in the recorded object, or in both, and `snapshot_never_invents_objects` must
reject each. A fourth, `RevokeReturnsAnotherRecipe`, makes a revoke return the issued entry with
another recipe, and `grant_is_replayable` must reject it.

Nine defects name one class: `GrantSubstitutesRequestedCapability`, `GrantSubstitutesRecipe`,
`GrantReturnsAnotherRecipe`, `GrantRecordsAnotherRecipe`, `RevokeReturnsAStranger`,
`RevokeReturnsAnotherRecipe`, `AcceptsInvalidOperationsOf`, `LeaksAnInvalidOperationOf` and
`RefusalRollsBackTheAddressOf`. The last three also name the call. Their witnesses run each of
them once for each of the five classes, against `snapshot_never_invents_objects` or
`grant_is_replayable`. So those two clauses fail a witness if they check only their first grant,
or only one class of invalid operation at a fresh or at an issued address. Every other defect runs
once, and no other clause is held to that.

Callers prepare grants. `GrantClosure` collects one owner's capabilities in attach order. Each
`PreparedGrant` knows its handle and its inverse, `backend_effect`, before `grant` runs. An
identity source that fails panics inside `prepare`, before anything is recorded or granted.
`grant_retry_returns_the_original_entry` holds a backend to spec 017's retry rules. The same
standing operation and kind returns the original entry. Another kind, owner or capability at an
issued address returns `IdentityConflict` and changes nothing. A grant at an applied or planted
address returns `IdentityConflict`, changes nothing and issues no record, whether it carries the
same payload, another owner or another capability. `fresh_backends_share_no_authority`, and the
last step of `snapshot_never_invents_objects`, check that a backend fresh from the factory
observes nothing an earlier instance did, so a backend that keeps its own durable record of grants
fails both. The first instance then revokes its grants in both orders.

`snapshot_never_invents_objects` also sends an invalid operation of every class through `grant`,
`apply` and `plant`, each at a fresh address and at an issued one. Each call must return
`InvalidOperation` and leave the account as it was just before that call. So must each revoke of
the handle of a grant refused at a fresh address, which returns `UnknownHandle`. The account must
not change between two of those calls either, so a refusal whose effect lands after it returned
fails, and so does a refused grant that drops the issued record at its address later
(`RefusedGrantDropsTheIssuedRecordLate`).
A later check therefore cannot hide what an earlier refusal changed. A refusal that clears or swaps
what stands at its address fails, and so does a revoke that tidies away residue. A private backend
that refuses another launch at an occupied broker endpoint, as spec 017 allows a real adapter to
do, passes every clause, so the relaunched broker in `repeated_grants_are_independently_removable`
must keep its own command and sockets.

A clause earns its place by being watched failing against a backend built to carry the defect it
names. The suite started at five, and the three added by task 009 came from asking what a broken
backend could still walk through: handles reused between live grants, `apply` and `apply_removal`
never called at all, and a handle revoked twice. That watching is now committed rather than
remembered: the private unit-test module `src/conformance/clauses_discriminate.rs` holds one
defective backend per defect and accepts only failures recorded by the clause's semantic checks.
A clause that stops discriminating fails there. Run those witnesses directly with
`cargo test -p plasmosome-testkit --lib conformance::clauses_discriminate`.

Owners are a plugin in one cell. `revoke_takes_its_owners_object` pairs two plugins in one cell
and one plugin in two cells, and `apply_and_removal_reach_the_universe` tries the same plugin in
another cell and another plugin in the same cell before every exact removal.

`graceful_timeouts_preserve_the_selected_holding` needs a backend that cannot drain. Its factory
must stall every graceful withdrawal of a holding owned by `conformance::stalled_owner()`, before
any release, while Force still succeeds; the fake factories arm this with
`FakeBackend::stall_graceful_drains_for_owner`. A future real factory must arrange the same stall
with real resources. The clause times the stalled holding out with a 50 ms and a zero deadline,
and each timeout must keep the holding and its issued record. It checks that a removal naming the
wrong owner is refused before any drain in both directions, and drains another plugin in the
stalled cell and the same plugin in another cell normally. It
withdraws the stalled holding and its equal peers in both orders, and runs each order twice,
draining every other holding under a 50 ms deadline and then under a zero deadline. The
stalled-first pass forces the stalled holding while every other holding stands; the peer-first
pass drains the others while it stands and forces it last. The clause runs once with granted
holdings through `revoke`, where a graceful `apply_removal` of the stalled grant must also time out
and keep its handle, and once with applied holdings through `apply_removal`, where the granted
peer is removed through `apply_removal` too and its handle must be retired. Each order, each
deadline, each entry point and each of those steps has its own defective backend, so deleting any
one of them turns its witness red, and so does moving the neighbour or the other-cell holding
across the forced withdrawal in either order. `repeated_grants_are_independently_removable`
removes a granted holding through `apply_removal` under both policies and checks that its handle
is retired, and each policy has its own defective backend.

The order a clause revokes in is part of what it proves. A detach replays a ledger in reverse push
order, so a backend that accepts revokes only in grant order was conformant right up until task 012
had `live_grants_hold_distinct_handles` walk both orders — it revokes its live set in reverse
first, then in grant order, and says in every failure which pass it came from.

**Passing the suite is not evidence of enforcement.** Clauses compare requested changes with
the backend's own account of the world. A backend can maintain a consistent in-memory inventory
without performing an operating-system operation; this interface provides no independent OS
oracle. Read "conformant" as "keeps its own books consistently", not "enforcing". Evidence of
enforcement must come from an end-to-end test that drives the real adapter and observes the
real world.

The second is the **integration layer**. A unit test exercises one crate. These exercise core,
backend and ledger together through their public APIs, with the outside world replaced only at
the seams — the first of them attaches a plasmid's capabilities, replays its ledger on detach,
and verifies the backend snapshot shows no residue.

`tests/artifact_verdicts.rs`, on macOS only, also runs the membrane. It has 38 rows. Most are
files it builds in the per-user temp directory, with the modes, flags and ACL entries each row
names. The others are a directory, a socket, two symlinks, two missing names, directories the
walk refuses, root's `/usr/bin/sudo`, and a path through a directory of another local user. Each
row's path goes through the controller's artifact check, core's `artifact::verify_artifact`, and
the membrane's, `runtime::artifacts::verify_artifact`. Both must give the row its verdict,
written in core's `ArtifactFault`, and name the row's path. Spec 023 §1 needs this:
a file the controller admits is one the membrane admits at launch, so `cell.new` refuses an
untrusted file before it allocates anything. One function, `as_core`, turns the membrane's fault
into core's. A row the two callers disagree on is a defect in one of them, not in the table.

The third is the **membrane differential** of spec 027 §6, a test fixture that no product path
uses. It holds the probe manifest and the comparator. `differential::load` reads the manifest
from JSON and refuses it, naming what is wrong, before anything is compared:

- a channel of spec 027 §5 with fewer than three probes, or a reach kind with fewer than three
  reach probes, none included;
- a rule kind §6 does not list, matched exactly, so `PID` is refused;
- a probe with no `workload_set`, or a null one, naming the probe: every probe must give the
  key, even when the list is empty;
- a rule on a field the probe lists in its `workload_set`;
- a probe id used twice, in the same channel or in two;
- two rules, two allowances, or a rule and an allowance, on one field;
- a rule, allowance or neutral name for an undeclared probe;
- neutral names without `/opt`, `/usr/local/bin` and a FUSE workspace at a path of its own, or
  with two mounts at one path;
- an unknown key at any level, a key repeated in one object at any depth, including inside an
  allowance's `cell_value`, or data after the manifest.

Refusing unknown keys matters because a misspelt `rules` or `allowances` would otherwise load as
an empty table. `Manifest::validate` checks a manifest built by hand the same way. Rules about
runs, such as two boots per lane, are not the manifest's, and are not checked here.

`differential::judge` compares the runs of the three lanes and gives the verdict. It first checks
the manifest with `validate`, then refuses runs it cannot judge: a lane with runs, but not exactly
two, two runs from one boot in one lane or across two, a run that observes an undeclared probe
or one probe twice, and a ruled field of a completed observation that holds something its rule
kind cannot hold. A probe with no completion marker, or one that failed before the step it
observes, is not run in that lane, and so is every probe in a lane with no runs. Any not run makes
the verdict not run. Every completed run is still judged, and compared with every completed run
of the other lane, so the verdict lists each failure it finds, even in a lane whose other run is
not run, and does not depend on the order of a lane's runs:

- noise, a field in which a lane's two runs differ;
- a reach probe that one open run and one absent run show equal;
- a lane, any lane, that does not show the manifest's neutral mount source, type, options, FUSE
  subtype or cgroup path, so a name copied from the cell into the reference lanes still fails;
- a field in which the cell lane differs from the absent lane, unless an allowance names that
  probe and field and the cell holds exactly its value.

The harness transcript is compared the same way, with no allowances and no rules. No rule
reaches it, because rules name probes, so a transcript key with the name of a ruled probe field is
still compared exactly. Spec 027 §6's verdict says why.

All three comparisons see each completed observation after normalization. A rule names a probe,
a field and a kind. In each run, `judge` replaces every value of that field with a placeholder
such as `<pid:2>`, numbered within that probe and run, so an extra value in one probe shifts no
other probe. How a kind is numbered decides which relations its placeholders keep:

- A value that names something, a process ID, boot ID, mount ID, peer group, device, inode,
  ephemeral port or resolved address, is numbered by first appearance across every field of the
  probe that has a rule of its kind. The fields are taken in the byte order of their names, and a
  list's values in their order. So equal values stay equal and distinct ones distinct, even in two
  fields: a leader that names another process of its list fails, and so do two files that report
  one inode in two fields. A parent mount ID is numbered with the mount IDs, and its placeholder
  is `<mount_id:N>`. Apart from that pair, two kinds never share numbers. So a peer group, the `N`
  of a mount's `shared:N`, `master:N` or `propagate_from:N`, is numbered apart from the mount IDs:
  it names a peer group, not a mount, and a mount and a peer group that share a number by chance
  are not related.
- A system time is numbered by rank within its field, earliest first, and equal times share a
  number, so time placeholders keep their order.
- A memory or load counter is numbered by first appearance within its field, so counters in two
  fields are not related.
- Every duration becomes `<duration>`, so durations keep no relation. A reach probe whose lanes
  differ only in the order of its durations or in which of them are equal does not discriminate,
  since timing is spec 006's.

A relation that changes can show as a difference in a later field of the same probe, instead of
or as well as the field that changed. In `a_leader_that_names_another_process_of_its_list_fails`,
only `leader` changes, and `pids` is the field that differs. In
`two_distinct_files_that_report_one_inode_fail`, the cell's two files in `distinct` share an
inode, and both `distinct` and `linked` differ: `linked`'s inodes did not change, but its numbers
moved. Times or counters whose relation matters go in one list field. A relation between two
probes is never compared, so a probe that needs one reports both values.

Every field no rule names is compared exactly, so nothing is normalized for how it looks, and a
field the workload set can have no rule. A ruled field must hold one value of the rule's kind or
a list of them, or the runs are refused, naming the lane, probe, field and kind:

- a process ID, mount ID, parent mount ID, peer group, device, inode or memory counter is a whole
  number. So a probe reports the `5` of `master:5` as a bare number, and a peer group rule
  refuses the tag text itself;
- an ephemeral port is a whole number up to 65535;
- a system time, duration or load counter is any number, but a list of system times must not mix
  integers with numbers serde_json holds as floats, since the two do not compare exactly. Floats
  are the numbers written with a fraction or an exponent, such as `5.0`, and, when serde_json
  reads JSON text, also `-0` and integers beyond `u64::MAX` or below `i64::MIN`. So `[5, 5.0]`,
  `[-0, 5]` and `[1e3, 1001]` are all refused. A duration or load counter list may mix, since it
  is not ranked. `5` and `5.0` are not equal JSON values, so in a load counter list `[5, 5.0]`
  gets two placeholders;
- a boot ID is a UUID, and a resolved address an IPv4 or IPv6 address.

Integer system times rank exactly across the whole `i64` and `u64` range, and the others as
`f64` values. Times that are equal JSON numbers share a number, so `-0.0` and `0.0` are one time.
No kind holds an object, so no field the workload set lists can sit under a ruled field. A number
the workload set, such as a time set with `utimensat`, can still sit in a ruled list, and keeping
it out is the manifest author's job. A rule checks a value's shape, not its meaning: a `pid` rule
on a numeric `errno` field makes 13 and 2 equal.

Within a probe, placeholders keep the relations above. So a reach probe whose open and absent
lanes differ only in a ruled field still discriminates when a relation differs, such as two equal
process IDs in one lane and two distinct ones in the other, and does not when only the numbers
differ.

`judge` takes typed runs and parses no bytes. A reader of the probe output a guest writes
belongs with the code that runs the lanes, and spec 027 §6 says which numbers that reader must
refuse.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, capabilities, prepared grants (`GrantClosure`, `PreparedGrant`), `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Thirteen clauses of the backend contract, each generic over `EnforcementBackend` |
| `src/conformance/clauses_discriminate.rs` | Private defective backends shown failing the clause that names each fault |
| `differential` | Spec 027 §6's probe manifest for the membrane differential, its refusals, and `judge`, which compares the lanes' runs and gives the verdict |
| `tests/` | The cross-crate scenarios, and where end-to-end tests will go once a cell boots |

Nothing here ships: the crate is `publish = false`, and a guard keeps it out of every other
crate's non-dev dependencies.

`ManifestBuilder::new(id, description)` requires the test's purpose text, and
`.tool(name, description)` adds a named tool with its sentence. The builder constructs trusted
test data directly; it is not a second TOML validator. The description scenario in
`tests/attach_detach_residue.rs` instead parses a real declaration and reads the resulting
descriptions through `ToolRegistry::lookup`, then verifies withdrawal removes them.

`session_file`, `uds_socket`, `proxy_map`, `broker` and `mount` return the capability of their
class with exactly the values given, neither checked nor normalized, and a fixed recipe that
passes `validate`:

| Factory | Default recipe |
| --- | --- |
| `session_file(path)` | empty contents, read-only, guest path `/run/plasmosome/session` |
| `uds_socket(path)` | upstream `/run/plasmosome/upstream.sock`, guest path `/run/plasmosome/guest.sock` |
| `proxy_map(host, route)` | TCP to `upstream.plasmosome.test` port 443, private addresses not allowed |
| `broker(name)` | command `/usr/libexec/plasmosome-broker`, control socket `/run/plasmosome/broker.control`, data socket `/run/plasmosome/broker.data` |
| `mount(source, target)` | read-only |

A test that needs another recipe or launch builds the capability itself. The suite's fixtures
build every capability through these five, and every universe operation through the private
`op_for`, so changing a recipe changes these bodies and `op_for`, not every fixture.

`plasmosome-backend`'s unit tests cannot use these. A dev-dependency on this crate links a second
build of the backend, and its `Capability` is a different type from the one those tests see
(E0308, "multiple different versions of crate"). So the backend keeps a test-only `fixtures`
module with the same five functions and `op`, the twin of `op_for`. Change both copies together.

Tests: `cargo test -p plasmosome-testkit`
