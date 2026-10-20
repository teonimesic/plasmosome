---
name: check-pipeline-health
description: Execute read-only pipeline statistics and return the actual scoped report to Main.
---

# Check pipeline health

Run in the repository cwd with Python 3.11+ and its existing gh/native access:

```sh
./tools/check-pipeline-health
```

The forge defaults to `gh repo view`; `--repo OWNER/REPO` makes that input explicit. Honor a native-operation pause before running. For separately authorized non-native collection, `--native-paused` issues no native commands and reports that source paused; it cannot release the pause.

Read the actual JSON and exit status. The envelope contains `observed_at`, `completed_at`, the trailing-hour `window`, `repository`, `collection_status`, `native`, `github`, `reviews` and `errors`. Preserve each source's scope, generation/consistency, age classification and unknowns. Review usage is not a merge-gate verdict or an inferred account balance.

The report does not mark outside PRs (spec 030 §1). Compare `github.open_prs` with each task's `external_ref` and each PR's `Steward:` comment, as spec 030 §6 says. A PR matching neither is waiting for a steward. One whose admission comment names a governing spec, with no task naming it, is orphaned unless its steward is still filing (spec 030 §6).

Exit 0 means successful collection within stated scopes, not healthy delivery or complete knowledge. Exit 2 means partial/unavailable collection or paused native data; command misuse also exits 2. Report failures/partial data as observed, never as zero or a fabricated successful report.

Keep the output in this Main session. This skill only executes and reads statistics; it does not allocate, claim, mutate, dispatch, request reviews or schedule timers. Main's role instructions govern decisions.
