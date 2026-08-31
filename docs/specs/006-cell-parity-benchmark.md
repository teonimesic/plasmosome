---
id: 006
title: The cell parity benchmark — cell vs host vs Docker
status: draft
intents: [002]
---

## Behavior

One benchmark, three lanes, one question. The same parallel Rust workload runs inside a cell, on
the bare host, and in a plain Docker image, on the same machine, with no resource limit set on
any lane. For each lane it reports median wall clock, achieved CPU utilization, and peak memory.
The question it answers: does a cell with unlimited resources perform close to the host — and if
not, how big is the gap and in which phase does it go.

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
of hashing work. Two phases are timed separately, because both are parallel workloads a
developer feels: `cargo build --tests` (compilation) and `cargo test` (the suite, under cargo's
parallel runner).

Why synthetic rather than a real crate's suite, which the intent suggests as the obvious
candidate: a real suite mixes IO with compute and drifts with every version, and its
dependencies drag networking or vendoring into all three lanes. A std-only crate runs offline,
identically, everywhere, and still exercises the scheduler the way a real parallel suite does.
Open question, owner's call: whether to add a second, real-crate workload later for realism —
the harness takes the workload directory as a parameter, so adding one is cheap.

### The driver: hyperfine 1.20.0

The lanes are whole processes in three different environments — one of them across a VM
boundary. That is exactly hyperfine's shape: warmup, repeated runs, median and spread per
command. criterion is in-process and cannot cross the VM boundary; a hand-rolled loop gives no
statistics.

**The version is pinned at 1.20.0**, and the exact version used is recorded with every results
file. This spec promises a stranger can rerun every lane from the results file alone, and
hyperfine's spread and outlier reporting may change between versions — an unpinned tool turns
that promise into a guess.

Each lane runs with at least 3 warmup runs discarded and at least 20 timed runs. Twenty is the
floor for the bootstrap interval below to say anything; three warmups is what it takes for the
filesystem cache to stop dominating the first result. Each lane also gets a no-op baseline
(`true`, `docker run <image> true`, cell-exec `true`) reported alongside — startup cost is shown,
never silently subtracted.

### Measuring resources, per lane

Two metrics carry the verdict: wall clock and CPU utilization. Memory is reported next to them
and decides nothing, for the reason below.

Utilization is (user + sys CPU seconds) / (wall seconds × host logical cores), from the same run
that produced the wall clock. Two caveats travel with every figure it produces, and both belong
in the results file:

- **The cores are not equal.** This machine has 12 performance cores and 4 efficiency cores with
  different throughput. Dividing by 16 puts 100% out of reach, and a lane the scheduler happens
  to place on more efficiency cores looks more utilized while doing less work.
- **An I/O-bound phase collapses the ratio toward zero.** That hits the two lanes crossing a VM
  filesystem boundary hardest — which is the exact comparison this benchmark exists to make, so
  the bias runs in the direction that matters most.

Peak memory is measured differently per lane, and the numbers are not comparable across lanes:

- **Docker and cell (Linux):** the cgroup's `memory.peak` under cgroup v2, or
  `memory.max_usage_in_bytes` under v1. That is a true tree-wide peak.
- **Host (macOS):** no tree-wide equivalent exists. Report `/usr/bin/time -l`'s figure labelled
  **largest single process RSS**, and say in the results file that it cannot be compared to the
  cgroup numbers.

Max RSS is not the peak memory of a process tree. `/usr/bin/time -l` and GNU `time -v` both
report a maximum across the tree, never a sum: four children of 300 MB report the same number as
one child of 300 MB, on macOS and inside Docker alike. For a parallel `cargo build` that number
is the largest single rustc, not what the lane asked of the machine — so it cannot answer the
question of how much of the host each lane used. Hence memory is reported with its method named,
and kept out of the pass/fail verdict.

Wall clock and utilization come from the same run:

- **Host (macOS):** `/usr/bin/time -l` around each phase — user, sys, and the single-process RSS
  figure above.
- **Docker:** GNU `time -v` inside the container around the same command, plus the cgroup peak.
  Docker Desktop's CPU and memory settings must be raised to the whole machine before the run and
  recorded with the results — otherwise "no limit" is false.
- **Cell:** GNU `time -v` inside the guest around the same command, plus the cgroup peak,
  returned over the exec channel.

Honesty about what is compared: on macOS, Docker runs inside its own Linux VM, so the Docker
lane is also a VM lane; and the host lane is Darwin while the other two are Linux. This is not a
pure kernel A/B — it is a comparison of the three environments a user of this machine can
actually choose, which is the decision the numbers exist to inform.

### What "close to the host" means

A threshold applied to noisy numbers means nothing, so the run protocol comes first. Every lane
reports its median, its inter-quartile range, and the full list of run times. **A lane whose
inter-quartile range exceeds 10% of its median is too noisy to judge.** Fix the environment and
run it again; never widen the band to fit the noise.

Two thresholds, both judged on a bootstrap 95% confidence interval on the ratio of medians rather
than on a bare comparison of two numbers:

1. **Floor (pass/fail):** the cell lane's median must not exceed the Docker lane's median. Docker
   is the incumbent a cell replaces, and being slower than the incumbent has to be explained
   before this benchmark is called passing. An interval that spans 1.0 means no difference was
   detected — that is neither a pass nor a failure, it is a run that was not decisive, and it is
   reported as such.
2. **Aspiration (reported, not gated):** the cell median is at most 1.20x the host median, judged
   by that same bootstrap interval on the ratio lying entirely below 1.20.

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

If the floor fails, the per-phase results say where the gap is: compile phase, test phase, or
startup baseline.

### Results

Dated markdown under `docs/benchmarks/`, one file per session, recording: machine model, chip,
core count, memory; macOS, Docker Desktop, and toolchain versions; Docker resource settings;
all raw hyperfine output; the derived utilization figures; and the two threshold verdicts.

## Contract

- The benchmark's name is `cell-parity`; its harness is a script under `benches/` invoked the
  same way for every lane, taking the lane and the workload directory as arguments.
- The workload crate is pinned: changing it starts a new longitudinal record and says so in
  `docs/benchmarks/`.
- Reported metrics per lane and phase: median wall clock, inter-quartile range, the full list of
  run times, utilization as defined above, peak memory with its measurement method named, and the
  no-op baseline. Only wall clock and utilization carry a pass/fail verdict.
- The hyperfine version is pinned and recorded with the results.

## Acceptance

- All three lanes run on one machine in one session with no resource limit set, and the Docker
  settings proving that are recorded.
- A `docs/benchmarks/` results file exists with every field the Design section lists, including
  the hyperfine version and, per lane, the full list of run times.
- Every lane ran at least 3 discarded warmups and at least 20 timed runs, and every lane's
  inter-quartile range is at most 10% of its median.
- The floor verdict (cell vs Docker) and the aspiration figure (cell vs host, against 1.20x) are
  both stated as bootstrap 95% confidence intervals on a ratio of medians, and an interval
  spanning 1.0 is reported as no difference detected rather than as a pass.
- Peak memory is reported per lane with its measurement method named, and no verdict depends
  on it.
- The harness and workload are in-repo and a stranger can rerun every lane from the results
  file's instructions alone.
- The gate in the root `AGENTS.md` is green.

## Blocked on

A runnable cell: VMM integration in `plasmosome-membrane`, a Linux guest image with GNU time,
and a controller implementing `cell.exec` from spec 001. Until those exist this spec stays
`draft` and files no task.
