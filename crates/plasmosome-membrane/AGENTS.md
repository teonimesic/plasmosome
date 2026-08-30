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
- **Never signal a pid after its terminal state was observed.** The pid may have been reused by
  an unrelated process. Terminal states are cached precisely so this cannot happen.
- **After `fork`, before `exec` or `_exit`, only async-signal-safe work.** The parent is
  multi-threaded; allocation, locking, and stdio in the child can deadlock. Implementations of
  the fork seam must not panic — an unwind runs the panic hook in the child.
- **Children are never tethered to the controller's lifetime.** The controller may crash and
  restart; cells keep running.

## Conventions

- No inline `//` comments. `///` contract docs on public items only.
- Prefer types that make misuse unrepresentable over documentation asking callers to be careful.

## Testing

`cargo test -p plasmosome-membrane`. Process-lifecycle tests must prove absence of orphans by
observation (a raw `waitpid` returning `ECHILD`), not by asserting the code path ran.
