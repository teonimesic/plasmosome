---
id: 016
title: Cells delegate work through explicit, temporary edges
status: draft
date: 2026-09-05
originator: Stefano
outcome:
---

A cell should not need every tool, operating system, or device required by the work it
coordinates. A Linux agent cell may ask a native cell to run Xcode, an emulator, Docker, or a
host diagnostic, and a Linux cell may provide a specialized service to another cell. The caller
should gain that ability only for the work it requested, without turning the callee into a
permanent part of its environment.

The connection between cells is a capability in its own right. It must be possible to remove it
while both cells remain alive: work already admitted may finish and return its recorded result,
while a new request is refused until a new connection is explicitly established. Cells must not
inherit one another's files, credentials, processes, devices, windows, input, or ambient host
authority merely because they can communicate.

Different cell names and membranes may express different capabilities and defenses. That lets a
Linux agent cell coordinate with a native performance cell while preserving the strongest
practical boundary around the native cell, and lets interactive cells operate concurrently
without seeing or controlling one another.

## Outcome

(filled in later)
