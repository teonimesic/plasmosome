---
id: 011
title: Isolation the model never has to know about
date: 2026-08-31
originator: Stefano
---

Models and harnesses are built on the assumption that they are running on real hardware. Making
one aware that it is in a constrained environment is like fighting the reinforcement learning it
went through on tool use: it spends tokens working around limits it has been told about instead
of doing the work. I do not want to pay for that, and I do not want to change how a harness
behaves in order to isolate it.

So what I want is an invisible membrane. It guarantees isolation without the model or the harness
being aware of it at all — not a limit the workload is asked to respect, and not a contract it
agrees to. Something it never has to know is there, and cannot be talked out of.

Invisible is the operative word, and it is what makes this more than a security statement. A cage
a model knows about changes what the model does. A cage it does not know about changes nothing
except what it can actually reach.

The consequence I want is that this holds for anything running in a cell: the harnesses I use
today, the ones I have not tried yet, and software that has never heard of Plasmosome. If the
isolation only holds when the workload cooperates, it is not isolation I can rely on.

Two pieces of work belong under this: the four-plasmid end-to-end run, and the first enforcement
backend that is not a fake. I want them built against it.

## Outcome

(filled in later)
