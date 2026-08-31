---
id: 006
title: The cell parity benchmark — cell vs host vs Docker
status: draft
intents: [002]
---

## Behavior

One benchmark, three lanes, one question. The same parallel Rust workload runs inside a cell, on
the bare host, and in a plain Docker image, on the same machine, with the same CPU capacity given
to every lane and no other resource limit set on any lane. For each lane it reports median wall
clock, CPU utilization, and peak memory; only wall clock decides pass or fail. The question it
answers: does a cell perform close to the host — and if not, how big is the gap and in which
phase does it go.

**This spec is blocked and stays `draft`.** The cell lane cannot be built: no runnable cell
exists. `plasmosome-membrane` has a fork/reap seam and a readiness probe, but no VMM
integration, no guest image, and no controller daemon to create a cell or exec into one
(spec 001's `cell.exec` is unimplemented). Measuring inside the guest additionally needs a Linux
guest with GNU time reachable over that exec channel. The host and Docker lanes are buildable on
a Mac today and the spec permits landing them first — but without the cell lane they answer none
of the question, so nothing here is filed as a ready task.

Everything below is decided now so that the day a cell boots, this becomes buildable without
another planning pass.

## Design

### The workload

A fixed crate at `benches/parity-workload/`: standard library only, no dependencies, with a test
suite of CPU-bound tests — at least twice as many tests as host cores, each doing a fixed amount
of hashing work. Three phases are timed separately, because each is a parallel workload a
developer feels: **cold compile** (`cargo build --tests` from an empty target directory), **warm
incremental** (`cargo build --tests` after one source file changed), and **test** (`cargo test`,
under cargo's parallel runner). The two compile phases answer different questions and each gets
its own run protocol, below.

Why synthetic rather than a real crate's suite, which the intent suggests as the obvious
candidate: a real suite mixes IO with compute and drifts with every version, and its
dependencies drag networking or vendoring into all three lanes. A std-only crate runs offline,
identically, everywhere, and still exercises the scheduler the way a real parallel suite does.
Open question, owner's call: whether to add a second, real-crate workload later for realism —
the harness takes the workload directory as a parameter, so adding one is cheap.

### Equal CPU capacity per lane

Every lane is pinned to the same number of logical CPUs, and `cargo build --jobs N` and
`cargo test -- --test-threads N` are set explicitly to that same N in all three lanes. N, and the
pinning method used for each lane, go in the results file.

Never let a lane take the default. `cargo build` and `cargo test` both default their parallelism
to the logical CPUs the lane can see, so a lane that sees fewer CPUs runs less of the work in
parallel and finishes slower for a reason that has nothing to do with the environment being
measured. A cell configured with 8 vCPUs against a 16-core host would be graded on how it was
configured, not on how it performs. Setting the two flags explicitly and identically removes that
from the comparison.

The pinning method differs per lane and is recorded rather than assumed: `--cpuset-cpus` for
Docker, the guest's vCPU count for the cell. macOS has no CPU affinity call to pin the host lane
with, so there the equal capacity rests on `--jobs` and `--test-threads` alone — a documented
limitation, stated in the results file next to the figures it affects.

### The driver: hyperfine 1.20.0

The lanes are whole processes in three different environments — one of them across a VM
boundary. That is exactly hyperfine's shape: warmup, repeated runs, median and spread per
command. criterion is in-process and cannot cross the VM boundary; a hand-rolled loop gives no
statistics.

**The version is pinned at 1.20.0**, and the exact version used is recorded with every results
file. This spec promises a stranger can rerun every lane from the results file alone, and
hyperfine's spread and outlier reporting may change between versions — an unpinned tool turns
that promise into a guess.

Each lane runs at least 20 timed runs of every phase. Twenty is the floor for the bootstrap
interval below to say anything. How many warmup runs precede them depends on the phase, and is
settled in the next section. Each lane also gets a no-op baseline (`true`,
`docker run <image> true`, cell-exec `true`) reported alongside — startup cost is shown, never
silently subtracted.

### Two compile phases, two protocols

Hyperfine's warmup runs populate the target directory. Give the cold compile phase any warmups
and the 20 timed runs after them find the crate already built, so they measure cache hits instead
of compilation. The two compile phases therefore get separate hyperfine invocations with separate
protocols, and both are reported.

**Cold compile.** `--warmup 0`, and `--prepare` removes the target directory before every run.
Warmups are invalid here by construction: a warmup leaves behind exactly the state this phase
exists to exclude. It answers how long a full build takes in this environment.

**Warm incremental.** `--prepare` touches one source file of the workload crate and leaves the
target directory in place. Warmups are allowed here, at least 3 discarded, because the
filesystem cache settling is noise rather than the thing being measured. It answers how long the
edit-and-build loop takes in this environment.

The test phase follows the warm protocol: at least 3 warmups discarded, target directory kept.

**Both warm phases need a populated target directory before their first timed run**, and neither
`--prepare` creates one. The driver runs an untimed `cargo build --tests` in the lane before
starting the warm incremental phase and again before the test phase, or else preserves the
artifacts the cold phase left behind. Without that, a lane starting from an empty target
directory measures a cold build on its first timed run and a warming cache on the next few, which
is the error the split exists to prevent.

Neither compile phase substitutes for the other. A cell can sit close to the host on incremental
builds and far off it on a cold one, and a developer feels both.

### Measuring resources, per lane

One metric carries the verdict: wall clock. CPU utilization and peak memory are reported next to
it and decide nothing, each for its own reason below.

Utilization is (user + sys CPU seconds) / (wall seconds × pinned logical CPUs), computed per run
from the user and system time recorded alongside that run's wall clock. Every timed run's user
and system time goes in the results file, and each lane reports the median of its per-run
utilizations. Computing it per run is what keeps it honest: a figure derived from a median wall
clock and a separately taken median CPU time can describe a run that never happened, because the
two medians need not come from the same run.

**Utilization is reported and never gated.** No threshold exists, because nobody knows yet what a
good number looks like on this machine. Any figure picked today would be invented, and the two
caveats below are enough on their own to make it arbitrary. Only wall clock decides pass or fail.
Once enough sessions have been recorded to say what normal is, a threshold becomes a decision
someone can make with evidence.

Two caveats travel with every utilization figure, and both belong in the results file:

- **The cores are not equal.** This machine has 12 performance cores and 4 efficiency cores with
  different throughput. Dividing by the pinned CPU count puts 100% out of reach, and a lane the
  scheduler happens to place on more efficiency cores looks more utilized while doing less work.
- **An I/O-bound phase collapses the ratio toward zero.** That hits the two lanes crossing a VM
  filesystem boundary hardest — which is the exact comparison this benchmark exists to make, so
  the bias runs in the direction that matters most.

Peak memory is measured differently per lane, and the numbers are not comparable across lanes:

- **Docker and cell (Linux):** the cgroup's `memory.peak` under cgroup v2, or
  `memory.max_usage_in_bytes` under v1. That is a true tree-wide peak. **Each phase of each run
  gets a fresh cgroup**: create it, run the phase inside it, read the peak, destroy it. Under
  cgroup v1, where the cgroup is reused instead, reset the counter by writing to
  `memory.max_usage_in_bytes` before the phase starts. A cgroup shared across phases holds a
  high-water mark, so every phase reports the largest peak any phase reached — the cold compile's
  peak would be read back as the test phase's, and both numbers would be wrong.
- **Host (macOS):** no tree-wide equivalent exists. Report `/usr/bin/time -l`'s figure labelled
  **largest single process RSS**, and say in the results file that it cannot be compared to the
  cgroup numbers.

Max RSS is not the peak memory of a process tree. `/usr/bin/time -l` and GNU `time -v` both
report a maximum across the tree, never a sum: four children of 300 MB report the same number as
one child of 300 MB, on macOS and inside Docker alike. For a parallel `cargo build` that number
is the largest single rustc, not what the lane asked of the machine — so it cannot answer the
question of how much of the host each lane used. Hence memory is reported with its method named,
and kept out of the pass/fail verdict.

Wall clock, user time and system time come from the same run:

- **Host (macOS):** `/usr/bin/time -l` around each phase — user, sys, and the single-process RSS
  figure above.
- **Docker:** GNU `time -v` inside the container around the same command, plus the cgroup peak.
  Docker Desktop's CPU and memory settings must be raised to the whole machine before the run and
  recorded with the results, so that the only cap on this lane is the `--cpuset-cpus` pinning
  every lane shares — otherwise "the same capacity as the others" is false.
- **Cell:** GNU `time -v` inside the guest around the same command, plus the cgroup peak,
  returned over the exec channel.

Honesty about what is compared: on macOS, Docker runs inside its own Linux VM, so the Docker
lane is also a VM lane; and the host lane is Darwin while the other two are Linux. This is not a
pure kernel A/B — it is a comparison of the three environments a user of this machine can
actually choose, which is the decision the numbers exist to inform.

### What "close to the host" means

A threshold applied to noisy numbers means nothing, so the run protocol comes first. Every lane
reports its median, its inter-quartile range, and the full list of run times.

**Every figure below is per lane and per phase.** The three timed phases are three different
commands, so a ratio built from a cold-compile sample in one lane and a warm sample in another
measures nothing. There is no combined number: three lanes times three phases gives nine medians,
nine inter-quartile ranges, and — for the two comparison verdicts — nine intervals. An aggregate
would also hide one noisy phase behind two quiet ones.

**A lane whose inter-quartile range exceeds 10% of its median, in any phase, is too noisy to
judge in that phase.** Fix the environment and run it again; never widen the band to fit the
noise.

Two thresholds, each evaluated separately for each phase, both judged on a bootstrap 95%
confidence interval on the ratio of medians rather than on a bare comparison of two numbers:

1. **Floor (pass/fail):** the cell lane's median must not exceed the Docker lane's median. Docker
   is the incumbent a cell replaces, and being slower than the incumbent has to be explained
   before this benchmark is called passing. **This verdict, and only this one, has a third
   outcome:** an interval that spans 1.0 means no difference was detected, which is neither a
   pass nor a failure but a run that was not decisive, and it is reported as such.
2. **Aspiration (reported, not gated):** the cell median is at most 1.20x the host median, judged
   by that same bootstrap interval on the ratio lying entirely below 1.20. That is the whole
   test — the spanning-1.0 rule above does not apply here. An aspiration interval of
   `[0.90, 1.10]` spans 1.0 and still lies entirely below 1.20, so it meets the aspiration; it
   spans 1.0 only because the cell and the host are close, which is the outcome this figure hopes
   for.

**The bootstrap, specified.** These choices decide whether an interval crosses a threshold, so
they are part of the contract rather than the harness author's discretion:

- **10,000 replicates.**
- **Resample each lane's per-run timings independently.** The lanes are not paired — there is no
  run-to-run correspondence between one environment and another, so a paired resample would
  invent one. Draw one resample of the cell lane's timings, one of the comparison lane's, take
  the ratio of their medians, and repeat.
- **Percentile interval** — the 2.5th and 97.5th percentiles of the 10,000 ratios.
- **A fixed random seed, recorded in the results file.** A stranger recomputing the interval from
  the recorded run times then gets the same interval, not a nearby one.

**1.20 is a product target, chosen to be argued about.** It is not derived from measurement and
nothing about the machine makes it the right number. Everything else in this section is method;
this one figure is a decision, and it is the one the owner should push back on.

The rejected alternative was "within the host lane's own noise band" — host median plus three
times the host lane's inter-run standard deviation. Its width is set by however noisy the host
happened to be that day, so the same formula grades the cell differently every session. Measured
on one machine, that formula gave a band 209.9% of the median on one workload and 103.2% on
another: the first admits a cell 2.1x slower than the host, the second demands the cell land
within 3.2%, which no VM will ever clear. One cold-cache outlier is the whole difference between
them. Three standard deviations is arbitrary on top of that — wall-clock time is right-skewed
with a hard lower bound, so the Gaussian intuition behind it does not apply.

The floor had a quieter version of the same problem. It compared two medians with no significance
test at all, while the run collected spread and never used it. The bootstrap interval is what puts
that spread to work.

If the floor fails, the per-phase results say where the gap is: cold compile, warm incremental,
test, or the startup baseline.

### Results

Dated markdown under `docs/benchmarks/`, one file per session, recording the environment, the
provenance of everything measured, and the numbers.

**Environment:** machine model, chip, core count, memory; macOS, Docker Desktop, and toolchain
versions; Docker resource settings.

**Provenance.** Without these, an input can change while every Results and Acceptance field still
looks satisfied, and two sessions stop being comparable with nobody noticing:

- the workload crate's commit hash, or a digest of its contents if it is not committed
- the Docker image digest, not its tag
- the cell guest image revision and the controller revision
- the exact commands run, per lane and per phase
- the pinning method and CPU count per lane, and the `--jobs` and `--test-threads` values
- the bootstrap seed and replicate count
- the hyperfine version
- the `time` implementation and its version per lane — `/usr/bin/time -l` on macOS, GNU `time -v`
  in the Docker and cell lanes

**Numbers:** all raw hyperfine output; every timed run's wall clock, user time and system time;
the per-run and median utilization figures; peak memory per lane with its measurement method
named; and the two verdicts.

## Contract

- The benchmark's name is `cell-parity`; its harness is a script under `benches/` invoked the
  same way for every lane, taking the lane and the workload directory as arguments.
- The workload crate is pinned: changing it starts a new longitudinal record and says so in
  `docs/benchmarks/`.
- Three timed phases per lane: cold compile, warm incremental, test. Cold compile runs with zero
  warmups and a `--prepare` that removes the target directory before every run; the other two
  discard at least 3 warmups and keep it.
- Every lane is pinned to the same logical CPU count, and `--jobs` and `--test-threads` are
  passed explicitly with the same value in all three lanes. No lane takes cargo's default.
- Reported metrics per lane and phase: median wall clock, inter-quartile range, the full list of
  run times with each run's user and system time, median utilization as defined above, peak
  memory with its measurement method named, and the no-op baseline. **Only wall clock carries a
  pass/fail verdict.**
- Peak memory is read from a cgroup created for that one phase of that one run and destroyed
  after it.
- The bootstrap is 10,000 replicates, resampled per lane independently, reported as a percentile
  interval, from a seed recorded with the results.
- The hyperfine version is pinned and recorded with the results, along with every other
  provenance field the Results section lists.

## Acceptance

- All three lanes run on one machine in one session, pinned to the same logical CPU count and
  given the same `--jobs` and `--test-threads`, with no other resource limit set — and the Docker
  settings proving that are recorded.
- A `docs/benchmarks/` results file exists with every field the Results section lists: the
  environment, every provenance field, and per lane and phase the full list of run times with
  each run's user and system time.
- Every lane ran all three phases with at least 20 timed runs each; cold compile ran zero
  warmups with the target directory removed before every run; warm incremental and test ran at
  least 3 discarded warmups. Every lane's inter-quartile range is at most 10% of its median.
- The floor verdict (cell vs Docker) and the aspiration figure (cell vs host, against 1.20x) are
  both stated as bootstrap 95% confidence intervals on a ratio of medians, computed as the Design
  section specifies. On the floor verdict only, an interval spanning 1.0 is reported as no
  difference detected rather than as a pass; the aspiration figure is judged solely on its
  interval lying entirely below 1.20.
- CPU utilization is reported per lane as a median of per-run figures, and no verdict depends
  on it.
- Peak memory is reported per lane with its measurement method named, and no verdict depends
  on it.
- The harness and workload are in-repo and a stranger can rerun every lane from the results
  file's instructions alone.
- The gate in the root `AGENTS.md` is green.

## Blocked on

A runnable cell: VMM integration in `plasmosome-membrane`, a Linux guest image with GNU time,
and a controller implementing `cell.exec` from spec 001. Until those exist this spec stays
`draft` and files no task.
