---
id: 015
title: Typed heartbeat observation and guarded application
status: draft
intents: [015]
---

## Behavior

The work-state layer exposes a typed, deterministic boundary for one heartbeat sweep. An
observation reads a repository's local projection and reconciliation inputs without mutation or
dispatch. An application accepts only that observation's still-fresh, authorized proposal and
routes it through the existing Spec 014 writer, ownership, lifecycle and publication gates.

The boundary is a library seam first; a CLI or recurring driver may use it, but neither is part of
this spec. A sweep is one bounded operation and is explicitly cancellable. It never approves an
intent, accepts a spec, treats stale state as current, or performs an external effect outside an
authorized action.

## Contract

### Observation

`heartbeat_observe(input) -> HeartbeatObservation` accepts a repository key, actor/session,
observed local generation, remote freshness envelope, UTC observation time, and the projected
intent/spec/task records needed for the lifecycle and link checks. The result contains:

- `observation_id`, deterministic from repository key, local generation and observation epoch;
- the complete freshness envelope from Spec 014;
- a reconciliation result with checked record keys, detected changes, and refusal diagnostics;
- zero or more `ProposedAction` values, each with a deterministic `action_id`, target document
  key, action kind, expected state version, expected generation, and required authority; and
- zero or more `PlannerRequest` values naming the exact missing or contradictory layer, with a
  deterministic request id and no mutation authority.

The result is read-only. A malformed lifecycle value, missing or mismatched spec/intent link,
unknown freshness, stale generation, unpublished mutation, or conflicting observation is a
refusal result, never an eligible action. The result distinguishes `refused`, `idle`, and
`proposed`; it does not use an empty proposal list to hide a refusal.

### Application

`heartbeat_apply(proposal, authority, cancellation) -> HeartbeatReceipt` requires the exact
`observation_id`, `action_id`, expected state version and expected generation from a prior
observation. Before mutation or effect it revalidates freshness, lifecycle/link gates, actor and
session authority, ownership token, and writer/publication fence through Spec 014. A changed
generation, state version, token, or proposal digest returns `stale` or `conflict` and performs
no mutation or external effect.

An application receipt contains the operation id, action id, observation id, target key, previous
and resulting state (when any), authority generation, outcome (`applied`, `already_applied`,
`refused`, `cancelled`, or `failed`), UTC timestamps, and a stable diagnostic code. Receipts are
append-only and idempotent: retrying an operation id returns the recorded result; a different
proposal under an existing operation id is a conflict. An interrupted operation is recoverable
only through the same operation id and fresh revalidation.

Planner requests are observations only. Applying one can record or dispatch a planner action only
when separately authorized by the existing ownership/publication contract; it cannot create an
intent, approve an intent, accept a spec, claim a task, or start work by inference.

### Cancellation and lifecycle

The operation lifecycle is `created -> observing -> observed -> applying -> completed`, with
`cancelled` and `refused` terminal outcomes. Cancellation is cooperative and checked before every
mutation/effect boundary; cancellation before that boundary produces no effect. Once an
authorized external effect has begun, cancellation records `cancel_requested` and waits for the
effect adapter's receipt; it does not claim that the effect was undone. Every terminal path is
reaped/closed and leaves a receipt or an explicit unreconciled recovery record.

Stable refusal codes include `stale_observation`, `unknown_freshness`, `unpublished_state`,
`state_version_conflict`, `generation_conflict`, `authority_conflict`, `malformed_status`,
`link_conflict`, `duplicate_action`, `cancelled`, and `missing_contract_input`. Codes are
machine-readable and the human rendering includes the target and reason without secrets.

### Determinism and isolation

Operation ids are caller-stable across retries. Action and planner-request ids are derived from
the repository key, operation kind, target key, observed generation and proposal digest; they do
not contain wall-clock time, process ids, or random values. One observation may propose many
independent targets, but an application never widens its authority to another target. No
ambient socket, credential, branch, descriptor, task claim, or process is acquired by observation.

## Acceptance

- A typed observation fixture returns the freshness envelope, checked keys, explicit state
  (`refused`, `idle`, or `proposed`), and deterministic action/planner-request ids.
- RED-first tests prove malformed status, missing/mismatched links, stale/unknown/unpublished
  freshness, and conflicting generation/version produce stable refusals before any effect.
- Applying a valid proposal revalidates the exact observation and routes through Spec 014's
  writer/ownership/publication seams; changed state, generation, authority, or proposal digest
  produces no mutation and no external effect.
- Replaying an operation returns one idempotent receipt, while reusing its id for another proposal
  is a conflict; receipts preserve outcome, generations, timestamps and target identity.
- Cancellation is tested before and at the effect boundary, distinguishing cancelled-before-effect
  from cancel-requested-after-effect without claiming rollback.
- Planner requests are non-authorizing and cannot approve, accept, claim, start, or dispatch work
  without a separate guarded action.
- The implementation remains limited to this typed contract and tests; recurring scheduling,
  provider execution, host operations, VMMs, GUI/input isolation, and intent approval remain out
  of scope.
