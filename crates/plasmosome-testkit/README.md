# plasmosome-testkit

Test support shared by the kernel crates: builders, the backend conformance suite, the
scenarios that cross crate boundaries, and the membrane differential's probe manifest and
comparator.

Three things live here. The first is the **conformance suite** — the behavioral clauses of the
`EnforcementBackend` contract, written once as functions generic over the trait. Every backend
is held to the same functions, unchanged; a backend that fails one is the thing that is wrong.
That is what makes the fake a model of enforcement rather than a hope about it.

`FakeBackend` and `CompositeBackend` over three fake leaves both pass all eleven clauses. The
composite failed three of the original clauses when it was first wired in, because it lost the
handle its leaf issued; task 008 fixed the backend rather than the clauses, which is the point of
holding every backend to the same functions.

The exact-grant clauses build expected objects from each request plus the issued identity, never
from a backend's returned owner or capability. Applied-object withdrawal keeps observations from
every other universe class standing. Dedicated defective backends alter each colliding resource
field, substitute an owner, remove cross-class state, and choose the wrong exact instance.

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
checks that a removal naming the wrong owner is refused before any drain in both directions, and
drains another plugin in the stalled cell and the same plugin in another cell normally. It
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

The harness transcript is compared the same way, with no allowances and no rules.

All three comparisons see each completed observation after normalization. A rule names a probe,
a field and a kind. In each run, `judge` replaces every value of that field with a placeholder
such as `<pid:2>`, numbered within that probe, field and run: by first appearance, or by order
for system times and durations. Equal values stay equal, distinct ones stay distinct, times keep
their order, and an extra value in one field shifts no other. Every field no rule names is
compared exactly, so nothing is normalized for how it looks, and a field the workload set can
have no rule. The field must hold one value of the rule's kind or a list of them, or the runs
are refused, naming the lane, probe, field and kind:

- a process ID, mount ID, parent mount ID, device, inode or memory counter is a whole number;
- an ephemeral port is a whole number up to 65535;
- a system time, duration or load counter is any number, but a list of system times or of
  durations must not mix integers with numbers written with a fraction or an exponent, such as
  `5` and `5.0`, since the two do not compare exactly;
- a boot ID is a UUID, and a resolved address an IPv4 or IPv6 address.

Integer times and durations rank exactly across the whole `i64` and `u64` range, and the others
by their total order. No kind holds an object, so no field the workload set lists can sit under a
ruled field. A number the workload set, such as a time set with `utimensat`, can still sit in a
ruled list, and keeping it out is the manifest author's job. A rule checks a value's shape, not
its meaning: a `pid` rule on a numeric `errno` field makes 13 and 2 equal.

Relations hold only within one field of one run. Two fields are numbered apart, so a relation
between them is not compared: a process list in `pids` and its leader in `leader`, or two files'
inodes in two fields, can change which values are equal and still pass. Values whose relation
matters go in one list field. Within a field, placeholders keep relations, so a reach probe whose
open and absent lanes differ only in a ruled field still discriminates when the relation differs,
such as two equal process IDs in one lane and two distinct ones in the other, and does not when
only the numbers differ.

Durations are ranked on purpose, like system times. Spec 027 §6 lists "times and durations the
system chose" together and says time placeholders keep their order. So a probe's verdict can turn
on timing order alone: a reach probe whose only difference is `[a, b]` in the open lane and
`[b, a]` in the absent lane discriminates. And a relation that changes between a lane's two runs,
such as two durations that swap order, is reported as noise.

`judge` takes typed runs and parses no bytes. A reader of the probe output a guest writes
belongs with the code that runs the lanes.

## What's inside

| Module | Holds |
| --- | --- |
| `builders` | `PlasmidManifest`, capabilities, `Grant` sequences, `Effect`s and `DesiredState` — a test states only what it is about |
| `conformance` | Eleven clauses of the backend contract, each generic over `EnforcementBackend` |
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
