# Security policy

To report a vulnerability in Plasmosome, use GitHub's private vulnerability reporting:
[open a private report](https://github.com/teonimesic/plasmosome/security/advisories/new). Please
do not open a public issue, pull request or discussion about it.

## How to report

The private report form is the only route for security reports. Say what you found, which commit
you tested, and the steps that show it. A proof of concept helps.

## Scope

- The crates and binaries in this repository.
- The cell boundary they build: what a workload in a cell can reach beyond its grants.
- Their supply chain: dependencies, the Rust toolchain, and CI.

## Not covered yet

What the security standard does not cover yet is listed under "Not covered yet" in its
[threat model](docs/specs/026-security-standard.md#threat-model).

## What to expect

Each report gets a fix, or a stated decision, before the vulnerability is disclosed publicly.
Please keep it private until then. No response time is promised yet, and there is no bug bounty.
