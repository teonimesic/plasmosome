# plasmosome-membrane — working notes

## What this crate is

The per-cell supervisor: owns the cell's VM process, its network path, and its broker daemons.
Runs on the host, one per cell. This is where enforcement lives.

## Hard rules

- **Every spawn is paired with a reap.** An un-reaped child is a process that kept capabilities
  it should have lost. Reaping happens on drop, not only on explicit kill.
- **Readiness is an answered query.** A broker that is alive but not serving is not ready; a
  socket file that exists proves nothing. Half-alive daemons are the failure mode this rule
  exists for.
- **Never signal after child identity was released.** A non-consuming terminal observation still
  retains authority to clean the managed group. A completed reap or measured `ECHILD` does not.
- **After `fork`, before `exec` or `_exit`, only async-signal-safe work.** The parent is
  multi-threaded; allocation, locking, and stdio in the child can deadlock. Implementations of
  the fork seam must not panic — an unwind runs the panic hook in the child.
- **Children are never tethered to the controller's lifetime.** The controller may crash and
  restart; cells keep running.

## Conventions

- Prefer types that make misuse unrepresentable over documentation asking callers to be careful.

## Testing

`cargo test -p plasmosome-membrane`. Process-lifecycle tests use a successful exact-child wait
to attribute a reap, `ECHILD` only to prove no waitable direct child remains, and descriptor EOF
to prove the fixture worker stopped.

A test that applies signal pressure must do two more things. It must **prove the pressure landed
where it was aimed** — a signal sent to the process can be handled on any thread, so a test of an
interrupted reap counts the interruptions the reaping thread actually took and fails if that count
is zero, rather than passing on a reap that was never interrupted. And it must **bound the
pressure**: a sender that fires until the reap finishes can keep interrupting it for as long as it
likes, which is a hang rather than a failure. Give each burst a fixed budget of signals.
