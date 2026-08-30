# plasmosome-backend

The enforcement seam. One interface between *deciding* a capability and *making it real*.

The controller says "this cell may reach api.example.com". Something must then edit a proxy's
allowlist, mount a directory, or install a credential — and that something differs by platform
and by capability. This crate is the line between the two halves.

It exists so the kernel's logic can be tested without a virtual machine. The in-memory backend
records what *would* have been granted; real backends do it. Both satisfy the same interface, so
lifecycle correctness, transaction rollback, and residue verification are tested at full speed
and full coverage, then run unchanged against the real thing.

The seam also carries an honest distinction: some grants are **hot** (a proxy map entry can
appear mid-run) and some are **generation-bound** (a VM's memory size cannot). The interface
makes a backend say which, rather than letting callers assume.

## What's inside

| Piece | Responsibility |
| --- | --- |
| `EnforcementBackend` | The trait: grant, revoke, and observe system state |
| `FakeBackend` | In-memory recorder — the test workhorse |
| `CompositeBackend` | Routes capability classes to the backend that owns them |
| Universe classes | What "system state" means for residue verification: sockets, mounts, processes, proxy entries, session files |

Tests: `cargo test -p plasmosome-backend`
