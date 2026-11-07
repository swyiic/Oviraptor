# Agent-loop Pi borrows — finishTurn, tool-set announcement, structured compaction

Status date: 2026-11-07. Scope: `packages/agent-host/` chat-agent turn loop.

Three loop-mechanics borrows from [earendil-works/pi](https://github.com/earendil-works/pi),
motivated by the recurring complaint that the chat agent does partial work,
names a gap, then **stops and hands it back** instead of finishing autonomously.

These are **loop mechanics only**. They grant no authority and do not touch the
safety model: per-action permission gates, the sandboxed no-net `workspace_run`,
the authorization wall, the no-auto-attested legal authorization, `max_turn_requests`,
stall detection (`tool_loop_stalled`) and cancellation all remain exactly as before.

| Task | What | State |
|------|------|-------|
| 1 | finishTurn completion policy (bounded force-continue) | **Implemented + tested** |
| 2 | Announce mid-conversation tool-set changes to the model | **Implemented + tested** |
| 3 | Non-destructive structured-summary compaction | **Designed-only (not built)** |

Pure decision logic lives in `src/agent-state.mjs` (no authority); the wiring is in
the `prompt()` turn loop in `src/provider.mjs`. Tests: `test/agent-loop-pi.test.mjs`.

---

## Task 1 — finishTurn completion policy (IMPLEMENTED + TESTED)

Pi ends a turn when the assistant message carries no tool calls, then consults a
`finishTurn` hook returning `{action:'continue'|'end'}`; `'continue'` guarantees
one more model request. Pi warns that an **unconditional** continue loops forever,
so the policy must be bounded and precise.

### Where it hooks
`src/provider.mjs`, inside the `for (let step = 0; step < stepLimit; step++)` loop,
on the clean end-of-turn path (`if (!turn.toolCalls.length) { … }`), **after** the
existing no-tool-calls guards (`shouldBlockEmptyCompletion`, the `toolHintGuard`
factual-claim nudge, `verifyBeforeEnd`, `qualifyUnverifiedEnd`) have each declined,
and immediately **before** the final `emitInner('done')` + `end_turn` return.
Because it runs last, it only ever catches the autonomous-incomplete case those
guards do not cover, and it never overrides them.

### The decision — `finishTurnDecision(...)` in `src/agent-state.mjs`
Returns `{ continue:true, reason, hint }` or `{ continue:false, reason }`. It forces
exactly one more model request only when **all** hold (checked in this order; the
first failing check is the decline `reason`):

1. not `cancelled` — cancellation always wins;
2. `answerMode === 'quality'` (autonomous runs; `fast`/simple mode is left alone);
3. forced-continue budget not exhausted (`forcedContinuesUsed < FORCED_CONTINUE_BUDGET`, a named const = **2**);
4. not a read-only / lookup turn (`!readOnlyTurn`);
5. the goal **expects a deliverable** — `deliverableExpected(goalText)`: an audit /
   pentest / vuln / exploit / decrypt / implement / fix / report / `record_finding`
   style task. A plain review, lookup or Q&A returns false here and is never forced;
6. the turn actually used a tool (`usedToolThisPrompt`);
7. **no deliverable was produced** — `producedDeliverable(...)` is false: no successful
   `record_finding`, and no successful `workspace_write`/`workspace_edit` to a
   report-like path (`.md`, or a path containing report/报告/finding/发现/writeup/summary);
8. the turn did real analysis/execution work — `didAnalysisWork(...)`: a successful
   `workspace_run`/`read`/`grep`/`glob`/`write`/`edit`, `scan_results`,
   `scan_fingerprint_read`, `inventory_*`, `exposure_overview`, `project_overview`, `journal_read`;
9. the model did **not** end by asking the user — `!looksLikeQuestionToUser(assistantText)`
   (trailing `?`/`？`, or confirmation/decision phrasing in CN/EN);
10. no authorization/scope boundary this turn — `!scopeBoundary`
    (`unavailableThisTurn.size > 0 || authorizationAttempted.size > 0`).

On a force, the loop injects `decision.hint` (a short, targeted "produce the
deliverable or take the next concrete step; only stop at a real boundary or a
genuine user decision" nudge) as a `user` message, increments `forcedContinues`,
extends `stepLimit = Math.max(stepLimit, step + 2)` (so the extra request is not
immediately cut by the step cap), and `continue`s.

### Why this is bounded and safe
* `forcedContinues` is tracked across the whole loop; at most `FORCED_CONTINUE_BUDGET`
  (2) forced continues per user turn — never unconditional.
* A new, later gate only: the pre-existing guards still fire first and still `continue`
  on their own terms.
* `max_turn_requests`, `tool_loop_stalled`, budget stops and cancellation are untouched —
  they return from earlier branches; the policy guards `!session.abort.signal.aborted`
  and passes `cancelled` into the decision so a mid-flight cancel never forces.
* `forcedContinues` / `forcedContinueReason` are surfaced in the `end_turn` and
  `max_turn_requests` metrics for observability.

### Tests (`test/agent-loop-pi.test.mjs`)
Pure-function unit tests (`finishTurnDecision (a)…(e)`, plus deliverable/read-only/
non-quality/scope/helper cases) **and** real-loop integration tests that drive
`OpenAiCompatibleRuntime.prompt()` with a stubbed model stream and a stubbed Host:

* `loop (a+c): autonomous audit with no deliverable forces one more turn up to the
  budget, then stops cleanly` — asserts 4 model requests (1 tool + 2 forced + 1 final),
  `stopReason:'end_turn'`, `metrics.forcedContinues === 2`, reason `incomplete_no_deliverable`.
* `loop (b): a turn that ends by asking the user a question is NOT force-continued` — 2 requests, `forcedContinues === 0`.
* `loop (b2): producing a deliverable (record_finding) ends the turn without forcing`.
* `loop (d): simple Q&A with no tools is unaffected`.
* `loop (e): cancellation during a tool call wins` — `stopReason:'cancelled'`, policy never runs.

---

## Task 2 — announce mid-conversation tool-set changes (IMPLEMENTED + TESTED)

Real failure this week: after a permission grant unlocked `workspace_run`/`workspace_write`
mid-conversation, the model kept concluding "I have no execution tools". Pi's lesson:
when the available tool set changes, tell the model.

### Where it hooks
`src/provider.mjs`, at the top of each loop step right after `grantedTools` is fetched
(and after the existing `restoredAppTools` re-grant note). It diffs the **raw granted
tool list** (real authority) — not the per-turn narrowed `tools` — against what the
model was last told (`announcedToolNames`, loop-local, `null` on the first step so the
first step only baselines and never announces).

### The diff — `toolSetAnnouncement(previous, current, {exclude})` in `src/agent-state.mjs`
Returns `{ added, executionAdded, message }` or `null`. `exclude` drops names the
`restoredAppTools` path already announced this step, so there is no double message.
The message names the newly-available tools and, when any are execution/write tools
(`workspace_run`/`write`/`edit`, `record_finding`), explicitly tells the model it now
has execution tools. Injected as a `_hostAuthorizationContext` `user` message (turn-local;
cleared at the next `prompt()` like the other host notes), counted in `toolSetAnnouncements`.

### Scope / honesty
Announcements are **within a single user turn** (the documented failure: a grant after an
in-turn authorization `continue`). Tool-set changes that happened in a *previous* turn are
already visible in `tools/list` at the next turn's first step (baseline), so they are not
re-announced — intentional, to avoid noise.

### Tests
* Unit: `toolSetAnnouncement` null-baseline, added/executionAdded diff, and `exclude`.
* Integration: `loop (Task 2): a mid-turn tool-set grant is announced to the model before
  the next request` — Host returns `[workspace_read]` on the first `tools/list` and
  `[workspace_read, workspace_run, record_finding]` afterwards; asserts
  `metrics.toolSetAnnouncements === 1` and that request #2's messages name `workspace_run`
  and carry the announcement.

---

## Task 3 — non-destructive structured-summary compaction (DESIGNED-ONLY, NOT BUILT)

> **This section is a design, not shipped code.** No part of Task 3 is implemented or
> wired. It is documented here so it can be picked up without re-deriving the cut-point rules.

### Why not built this session
The brief is explicit: *do NOT half-build this; a smaller fully-working Task 1 beats three
half-built tasks.* `src/memory.mjs` already contains a heavily-tuned, test-covered
`compressSessionMessages` (byte-budget driven; folds old tool results into harness facts;
already pairs an assistant `tool_calls` message with its `tool` results into one "protocol
unit" so a drop never orphans a result — see the `units` loop in `compressSessionMessages`).
A correct structured-summary compactor is a large addition that would either duplicate or
destabilize that path and its tests. Shipping it untested would violate the honesty bar, so
it is left as a concrete design.

### Goal
For very long autonomous runs, replace the oldest turns with **one non-destructive
structured summary** (sections below) instead of lossy byte-clipping, while **never
splitting a `tool_call` from its `tool_result`** at a cut point.

### Summary shape (one synthetic `user`/host message, literal/untrusted, grants no authority)
```
## Goal            — the user's durable objective (from harness.goal / first user turn)
## Constraints     — scope/authorization/budget limits stated so far (never widened)
## Progress
   - Done          — completed, evidence-backed steps (confirmed facts)
   - InProgress    — started but unfinished
   - Blocked       — blocked items + the specific blocker (permission / scope / tool error)
## Key Decisions   — choices made and why (tool/approach selections, rejected paths)
## Next Steps      — the open plan tail
## Critical Context— IDs, paths, revisions, task/run IDs that later steps need verbatim
cumulative: readFiles[], modifiedFiles[]   — unioned across all compactions, never dropped
```
`readFiles` / `modifiedFiles` accumulate across every compaction (union, de-duplicated) so a
later turn can still see what was touched even after the raw tool turns are gone.

### Cut-point rules (the crux — must hold exactly)
1. **Never split a protocol unit.** A cut point may fall only *between* units, where a unit =
   an assistant message with `tool_calls` **plus every** `role:'tool'` message whose
   `tool_call_id` matches one of those calls. Reuse the existing `units` grouping in
   `compressSessionMessages` as the authority for unit boundaries.
2. **Keep the system message** (index 0) verbatim.
3. **Keep the most recent `keepRecent` units** verbatim (unsummarized window).
4. **Keep the last user message** verbatim even if older than the window (anchors the turn).
5. Summarize only whole units **older** than the window; emit the structured summary in
   their place, once, at the head (after system).
6. The summary is **additive/non-destructive**: facts/decisions/critical-context are derived,
   never fabricated; IDs/paths are copied verbatim; nothing in `Constraints` is relaxed.
7. If the structured summary itself still exceeds the byte budget, clip **within a section's
   body text** (longest section first) — never by dropping a `Critical Context` ID or by
   breaking a kept protocol unit.

### Proposed API (additive; does not replace `compressSessionMessages`)
```js
// src/memory.mjs
export function structuredSummaryCompact(messages, {
  keepRecent = SESSION_MESSAGE_LIMITS.keepRecent,
  maxTotalBytes = SESSION_MESSAGE_LIMITS.maxTotalBytes,
  priorSummary = null,        // merge with the previous structured summary (cumulative files)
} = {}) {
  // → { messages, summary, readFiles, modifiedFiles, compressed }
}
```
Wire it in `provider.mjs` only where `compressSessionMessages` is called for the *soft
post-tool-wave compaction of very long runs* (the `session.messages.length > 12 ||
bytes > 48_000` branch), gated behind a size threshold well above the current one, so short
turns keep today's behavior unchanged.

### Tests to add before it is considered built
* A cut point never lands inside a unit: assert no emitted request has a `role:'tool'`
  whose `tool_call_id` lacks its assistant `tool_calls` in the same output.
* Every section present; `Critical Context` IDs preserved verbatim.
* `readFiles`/`modifiedFiles` accumulate across two successive compactions (union, no loss).
* Output respects `maxTotalBytes`; over-budget clips section bodies, never units or IDs.
* `Constraints` are never widened vs the input.

### Interim
Until Task 3 ships, `compressSessionMessages` continues to handle long runs (it already
preserves tool-call/result pairing and folds old results into harness facts), and Task 1's
finishTurn nudges operate on top of it unchanged.
