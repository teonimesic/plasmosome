---
id: 016
title: Cells delegate work through explicit, temporary edges
status: draft
date: 2026-09-05
originator: Stefano
outcome:
---

A cell should not need every tool, operating system, or device required by the work it
coordinates. When I allow it, a Linux agent cell may ask a native cell to run Xcode, an emulator,
Docker, or a host diagnostic, and a Linux cell may provide a specialized service to other cells.
The caller should gain that ability only for the work it requested, without turning the callee
into a permanent part of its environment.

The connection between cells is a capability in its own right. It must be possible to remove it
while both cells remain alive, and the removal must hold without either cell's cooperation. Work
already admitted may finish and return its recorded result, while a new request is refused until
a new connection is explicitly established. Work that keeps running after its connection is
removed still has an owner and can still be stopped. What it did, and anything it left behind, is
recorded and traced back to the cell that asked for it. Cells must not inherit one another's
files, credentials, processes, devices, windows, input, or ambient host authority merely because
they can communicate. Two cells that use the same service cell must not reach each other's data
through it.

Different cell names and membranes may express different capabilities and defenses. That lets a
Linux agent cell coordinate with a native performance cell while keeping the strongest practical
boundary around the native cell. That boundary is weaker than a Linux cell's. I accept that, as
long as I am told. For each delegation, Plasmosome must say which cell and which membrane do the
work, what that boundary does not stop, and what host authority the work can reach. A native cell
must never be presented as the isolation intent 011 describes.

Interactive cells that run at the same time without seeing or controlling one another are a
separate want. This intent does not cover them, and delegation working is not evidence for them.

## Outcome

(filled in later)
