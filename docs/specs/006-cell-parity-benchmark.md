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

### The driver: hyperfine

The lanes are whole processes in three different environments — one of them across a VM
boundary. That is exactly hyperfine's shape: warmup, repeated runs, median and spread per
command. criterion is in-process and cannot cross the VM boundary; a hand-rolled loop gives no
statistics. Each lane: one warmup run, then ten measured runs. Each lane also gets a no-op
baseline (`true`, `docker run <image> true`, cell-exec `true`) reported alongside — startup cost
is shown, never silently subtracted.

### Measuring resources, per lane

Utilization is defined as (user + sys CPU seconds) / (wall seconds × host logical cores), from
the same run that produced the wall clock. Peak memory is max RSS.

- **Host (macOS):** `/usr/bin/time -l` around each phase — it reports user, sys, and max RSS.
- **Docker:** GNU `time -v` inside the container around the same command. Docker Desktop's CPU
  and memory settings must be raised to the whole machine before the run and recorded with the
  results — otherwise "no limit" is false.
- **Cell:** GNU `time -v` inside the guest around the same command, returned over the exec
  channel.

Honesty about what is compared: on macOS, Docker runs inside its own Linux VM, so the Docker
lane is also a VM lane; and the host lane is Darwin while the other two are Linux. This is not a
pure kernel A/B — it is a comparison of the three environments a user of this machine can
actually choose, which is the decision the numbers exist to inform.

### What "close to the host" means

No invented percentage. Two derived thresholds, checked against medians from the same session:

1. **Floor (pass/fail):** the cell lane is no slower than the Docker lane. Docker is the
   incumbent a cell replaces; slower than the incumbent means the gap must be explained before
   this benchmark is called passing.
2. **Aspiration (reported, not gated):** the cell median is within the host lane's own noise
   band — host median plus three times the host lane's inter-run standard deviation.

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
- Reported metrics per lane and phase: median wall clock, spread, utilization as defined above,
  max RSS, no-op baseline.

## Acceptance

- All three lanes run on one machine in one session with no resource limit set, and the Docker
  settings proving that are recorded.
- A `docs/benchmarks/` results file exists with every field the Design section lists.
- The floor threshold verdict (cell vs Docker) and the aspiration figure (cell vs host noise
  band) are stated in the results file.
- The harness and workload are in-repo and a stranger can rerun every lane from the results
  file's instructions alone.
- The gate is green: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo fmt --all -- --check`, `./.githooks/provenance-guard`.

## Blocked on

A runnable cell: VMM integration in `plasmosome-membrane`, a Linux guest image with GNU time,
and a controller implementing `cell.exec` from spec 001. Until those exist this spec stays
`draft` and files no task.
