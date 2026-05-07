# clms archaeology — design (v2 final)

> **status:** v2 supersedes v1. v1 design is at git ref `f48d235`; v1 impl
> at `235bdf4`. v2 corrects two structural errors v1 carried forward
> through the first v2 draft (auto-verification, role-biased debate).

## the lies v1 was telling

1. **transcribe-as-claim.** v1 emitted one claim per commit and one per mb
   entry. result: ~30 of 35 outputs were noise. claims live in present tense
   with stake; commits live in past tense as events. wrong category.

2. **enumeration without bound.** any source that scales output with
   codebase/history size generates noise faster than curation absorbs it.

3. **auto-verification.** v1 wrote `state: verified` claims with evidence
   stamped from documentary signals it never ran. that's the same "we know
   this was true" lie at any scale. v2-draft1 carried this forward; oracle
   review caught it.

v2 fixes all three by construction.

## first principles

| principle | consequence |
|---|---|
| claim ≠ fact | enumeration sources score zero; intent-encoded signals score high |
| stake is intentional | language with stake-encoding wins (must, always, never, invariant) |
| **archaeology never verifies** | phase 3 always writes `state: pending`. promotion is `clms verify`'s job |
| bounded output | hard cap, default `--max=10`, ceiling 50 |
| drop is default | survival requires affirmative argument under token budget |
| transcript over verdict | every debate decision logged for replay/override |

archaeology is a **candidacy engine**, not a verification engine. that
distinction is the whole game.

## the three phases

```
phase 1   harvest    rust, in clms              ≤ N candidates
phase 2   debate     orchestrator-agnostic       drop-is-default judge
phase 3   commit     rust, in clms               survivors → pending claims
```

clms owns phases 1 and 3. clms does NOT orchestrate phase 2. it defines a
JSON protocol both ends respect, and ships a reference implementation for
[`pi-subagents`](https://www.npmjs.com/package/pi-subagents). any
orchestrator that respects the protocol works equivalently.

## phase 1 — harvest

`clms archaeology suggest [--max N] [--source ...] > candidates.json`

### signal taxonomy (in priority order)

| kind | source | example signal | example claim |
|---|---|---|---|
| `clms-claim-annotation` | code-comment annotation | `// clms-claim: ledger is append-only` | "ledger is append-only" |
| `test-name-invariant` | test fn names | `test_no_import_cycles` | "no import cycles exist in core/" |
| `type-marked` | `Final`, `frozen`, `readonly` | `Final[List[str]]` | "X is immutable after init" |
| `mb-verify-task` | mb tasks | "verify totals match counterparty" | "totals match counterparty within $0.01" |
| `cm-structural` | cm queries (on demand only) | `cm cycles core/` | "no cycles exist in core/" |
| `git-revert` | explicit reverts | `Revert "..."` | refute edge only, never new claim |

`clms-claim-annotation` is the highest-density signal because it requires
explicit intent to write. format:

```rust
// clms-claim: <one-sentence falsifiable assertion>
// clms-evidence: <method=...> [cmd=...] [ref=...]
fn append_to_ledger(...) { ... }
```

```python
# clms-claim: this function is pure (no IO, no global state)
def transform(record: Record) -> Record: ...
```

scanning for `// clms-claim:` and `# clms-claim:` is a deterministic, fast,
zero-false-positive signal. it's also a quiet contract: agents writing code
declare invariants inline; archaeology harvests them without prose-mining.

### intent surfaces (cold-start handling)

the harvester reads from TWO surfaces, both emitting `clms-claim-annotation`
candidates:

| surface | written by | location | persistence |
|---|---|---|---|
| in-source annotations | humans | `// clms-claim:` / `# clms-claim:` in `*.rs/.py/.ts/.go/...` | durable, version-controlled |
| proposals manifest | agents | `.archaeology/proposals.json` | ephemeral, regenerable |

rationale: a fresh repo has no annotations, so `suggest` would return zero
candidates. requiring humans to manually annotate before any archaeology
value is delivered is a cold-start failure mode. the proposals manifest
lets a generative subagent (`.pi/agents/clms-proposer.md`) seed the
harvester without polluting source code with potentially-bad comments.

the pair is adversarially correct:

- **proposer** = generative, low-rigor, may be sloppy
- **judge** = discriminative, drop-by-default, ruthless

bad proposals get dropped at phase 2 instead of surviving as dead comments
in the codebase. if a proposal IS strong and you want it to persist across
future re-harvests, cherry-pick it into source as a real `// clms-claim:`
comment after `clms verify` confirms it.

#### proposals.json schema

```json
{
  "version": "archaeology/v2",
  "proposals": [
    {
      "text": "ledger writes are append-only",
      "where": "src/store.rs:142",
      "snippet": "// (proposal) clms-claim: ledger writes are append-only",
      "suggested_evidence": [
        {"method": "code-test", "cmd": "cargo test test_append_only"}
      ]
    }
  ]
}
```

- `text` and `where` required; `snippet` and `suggested_evidence` optional
- candidate_id is hash-stable across re-runs (kind + text + where), so
  judge transcripts re-attach if you regenerate proposals.json with the
  same content
- harvester refuses on missing fields or wrong schema version (single-line
  json envelope under `--format ai`)
- candidates from this surface have `source_meta.file = ".archaeology/proposals.json"`,
  so the judge can apply origin-aware skepticism if desired

#### the no-human-in-the-loop default

four-step pipeline, zero chat-review gates:

```bash
# (orchestrator runs clms-proposer agent: reads code, writes proposals.json)
clms archaeology suggest -o candidates.json
# (orchestrator runs clms-judge agent: reads candidates, writes survivors.json)
clms archaeology commit --from-plan survivors.json
```

the agent pair (proposer + judge) IS the review. clms remains orchestrator-
agnostic; pi-subagents is the reference implementation but any orchestrator
that respects both agents' contracts works.

### bounded-N rule

```
default --max=10      hard cap
absolute ceiling      50    (refuses larger; defeats the purpose)
```

if extracted candidates exceed N, harvester ranks by signal strength and
emits top-N. tie-break by source priority (table above, top to bottom).

### candidate schema

```json
{
  "version": "archaeology/v2",
  "generated_at": "2026-05-06T22:00:00Z",
  "harvester": {
    "max": 10,
    "actual": 7,
    "extracted_total": 7,
    "truncated": 0,
    "sources_enabled": ["clms-claim-annotation"],
    "from_source": 4,
    "from_proposals": 3
  },
  "candidates": [
    {
      "id": "c-7f3a",
      "kind": "clms-claim-annotation",
      "text": "ledger writes are append-only",
      "stake_signal": {
        "where": "src/store.rs:142",
        "snippet": "// clms-claim: ledger writes are append-only"
      },
      "suggested_evidence": [
        {"method": "code-test", "cmd": "cargo test test_append_only", "note": "advisory; not run by archaeology"}
      ],
      "source_meta": {"file": "src/store.rs", "line": 142},
      "created_at": "2024-08-12T19:31:04Z",
      "debate": null
    }
  ]
}
```

`id` is `c-` + first 4 hex of `blake3(text + kind + stake_signal)`. stable
across re-harvests; debate transcripts re-attach to the same id on rerun.

`suggested_evidence[].note` makes the advisory nature explicit. archaeology
does not run these; they are hints for `clms verify` later.

`debate: null` until phase 2 fills it in.

### chronological ordering

candidates are sorted **oldest first** by `created_at`. provenance:

| surface | created_at source |
|---|---|
| in-source annotation | `git blame` author-time of the line where the marker appears |
| proposals.json row | mtime of `.archaeology/proposals.json` |
| uncommitted annotation | `null` (sorts to end) |
| no git repo / unreadable | `null` (sorts to end) |

rationale: claims earn their stake by surviving subsequent edits. an
annotation that's lived through three years of refactors has demonstrated
stake that yesterday's hasn't. when bounded-N truncates, oldest survives —
that's the intentional FIFO bias.

file:line is the tiebreak when timestamps are equal or both null.

`debate: null` until phase 2 fills it in.

## phase 2 — debate

one pass. one agent. drop is default.

```
input    candidates.json + K cap (max survivors, from --keep flag)
agent    clms.judge   (pi-subagents reference impl; or any orchestrator)
budget   ~600 tokens for N=10 candidates
output   survivors.json with debate.judge populated and keep:bool per row
```

### why one pass, not three

draft 1 of v2 had three passes (advocate-per-claim → prosecutor → judge).
oracle review caught the structural problems:

- "advocate" role is sycophantic regardless of prompt; naming biases
  behavior. "drop is default" doesn't override "you are the defender."
- 3 stochastic hops compound randomness without adding signal.
- at bounded-N=10, parallel advocacy doesn't buy wall-clock; one judge
  reads all candidates in one context window comfortably.
- cross-comparison (catching subsumption) is a *feature* of stake-judgment,
  and per-claim parallel advocacy actively prevents it.

single drop-by-default judge captures the same adversarial pressure with
~20% the tokens, half the stochastic surface, and no role-bias.

### why the parent must NOT inline-judge

the most common failure mode in early dogfooding: the parent agent reads
candidates.json itself, decides keep/drop, and writes survivors.json
directly. this looks faster but defeats the design — an agent grading
its own context-influenced harvest is biased toward keeping things, since
it just spent tokens fetching them. the discrimination signal collapses.

the orchestration invariant: **the parent ALWAYS spawns `clms.judge`**.
the judge runs in fresh context (`inheritProjectContext: false`,
`inheritSkills: false`) so it sees only the candidates.json text and
the agent prompt — no parent history, no prior arguments, no
context-influenced bias.

### install

for the orchestrator to discover the agent, it must be at user scope
(or project scope when cwd is the project). easiest path:

```bash
clms install-agents          # writes ~/.pi/agent/agents/clms/{judge,proposer}.md
clms install-agents --force  # overwrite when you upgrade clms
```

verify discovery:

```typescript
subagent({ action: "list" })  // expect clms.judge + clms.proposer
```

### the canonical spawn (pi-subagents reference)

```typescript
subagent({
  agent: "clms.judge",
  task: `apply drop-by-default judgement.
    INPUT: ${ABS_PATH}/candidates.json
    OUTPUT: ${ABS_PATH}/survivors.json
    return: "wrote N survivors, M cuts".`,
})
```

the judge has `tools: read, write`; it persists survivors.json itself
instead of streaming JSON back through the parent (more reliable than
asking the parent to faithfully serialize).

### the agent file (.pi/agents/clms-judge.md)

shipped in the clms repo and installed to user scope by `clms install-agents`.
the runtime name is `clms.judge` (frontmatter: `name: judge`, `package: clms`).

```markdown
---
name: judge
package: clms
description: drop-by-default cut and ranking of archaeology candidates
inheritProjectContext: false
inheritSkills: false
tools: read, write
---

clms is an append-only ledger of falsifiable claims with stake. archaeology
backfills candidate claims from existing repo signals. claims that don't
represent monitorable invariants pollute the ledger and dilute every future
`clms context` call. your job: be the gate that prevents that.

You receive a candidates.json with N proposed claims, each with stake-signal
evidence and source metadata. You also receive a K cap (max survivors).

Default verdict for every candidate: DROP. Survival requires you to
articulate, in ≤80 tokens per survivor:

1. who has stake in this claim being true (downstream users? CI? finance?
   security? if "no one specific," vote drop)
2. what would change in their behavior if the claim flipped to false
3. why this is monitorable as an *invariant*, not a one-time fact

If you find yourself arguing "this seems true and falsifiable," that is NOT
sufficient. Many true falsifiable facts are not claims. Stake is the test.

Cross-compare across all candidates. If two are subsumed by one, keep the
parent and drop the children. If a candidate is better-tracked elsewhere
(type system, CI assertion, lint rule), drop it.

Total budget: ~600 tokens for N=10. If your budget binds before you've
covered every candidate, remaining defaults to drop.

Output (single-line JSON, written to survivors.json):
{
  "version": "archaeology/v2",
  "judge": {
    "survivors": [{"id":"c-XXXX","rank":N,"rationale":"..."}],
    "cuts": {"c-YYYY":"<≤30 token reason>"},
    "tokens_used": N
  },
  "candidates": [<original candidates with debate.judge filled and keep:bool>]
}

Hard rules:
- never exceed K survivors
- never invent new candidates
- never propose new evidence (use the candidate's suggested_evidence as-is)
- never argue for upgrading anything; commit always writes pending
- if every candidate fails the stake test, output zero survivors. that is
  a valid outcome — it means the harvest was noise and the user should
  tighten signal rules or accept that this repo has no archaeology yield
```

### the protocol contract (orchestrator-agnostic)

any orchestrator that wants to substitute for pi-subagents must respect:

1. **input:** read `candidates.json` matching the schema above.
2. **output:** write `survivors.json` containing the original candidates,
   with each row's `debate` field populated (`{"judge": {...}}`) and a
   `keep: bool` per row.
3. **drop is structural.** `keep: true` requires affirmative justification
   in the row's `debate.judge.rationale`. orchestrators must not default to
   keep.
4. **K is post-hoc enforced.** `clms archaeology commit` rejects survivor
   files where `count(keep:true) > K`. orchestrator failure to respect K
   is caught by clms, not assumed.

with these four invariants, a hand-written python driver, a different
agent runtime, or a human curator opening candidates.json in vim are all
first-class.

## phase 3 — commit

`clms archaeology commit --from-plan survivors.json [--keep K]`

reads the file, validates, writes each `keep: true` row as a **pending**
claim with `archaeology_meta` populated.

### invariants (refusal conditions)

clms refuses to commit if:

- `debate` is null on any `keep: true` row (no debate happened — there is
  no `--allow-no-debate` flag; manual users go through `clms add`)
- `keep: true` but `debate.judge.verdict` not present
- `count(keep:true) > K`
- candidate `id` collides with existing `archaeology_meta.candidate_id` in
  storage (re-runs are idempotent; same id → skip with log line)

### claim shape on disk

```json
{
  "schema_version": "...",
  "agent": "archaeology",
  "session": "backfill-<rfc3339-ts>",
  "text": "ledger writes are append-only",
  "state": "pending",
  "evidence": [],
  "archaeology_meta": {
    "candidate_id": "c-7f3a",
    "kind": "clms-claim-annotation",
    "stake_signal": {"where": "src/store.rs:142", "snippet": "..."},
    "suggested_evidence": [
      {"method": "code-test", "cmd": "cargo test test_append_only"}
    ],
    "debate_transcript_ref": ".archaeology/<session>/c-7f3a.json",
    "judge_rationale": "<survivor rationale from debate>",
    "judge_rank": 4
  }
}
```

`archaeology_meta` is a new optional field on `Claim`. additive, non-breaking.

`evidence: []` on first write. promotion happens through normal
`clms verify <id> --method ... --ref ...`. the suggested_evidence is a
hint, not a write.

debate transcripts archive at **`.archaeology/<session>/<candidate-id>.json`**
at repo root (NOT inside `.claims/`, to avoid reindex-glob footguns).

## flags (final)

```
clms archaeology suggest [opts]
  --max N                hard cap on candidates (default 10, ceiling 50)
  --source <kind>        repeatable; defaults to all signal kinds except cm
  --cm-queries <list>    comma-separated cm queries (only with --source cm)
  --output <path>        candidates.json path (default stdout)

clms archaeology commit --from-plan <path>
  --keep N               cap on committed claims (default 8)

clms archaeology purge --session <stamp> [--agent <a>]
  cleanup utility for v1 spew or aborted runs.
  matches on agent + session, removes claim files, regenerates index.

REMOVED in v2 (errors out with migration hint):
clms archaeology                       (v1 auto-write)
clms archaeology --dry-run             (use `suggest` instead)
clms archaeology --no-mb               (use `--source` selection instead)
clms archaeology --since <sha>         (filter at harvest time, not v1's
                                        whole-history transcribe)
```

removed commands print:

```
{"error":"archaeology v1 removed. use `clms archaeology suggest` then `clms archaeology commit --from-plan`. see docs/archaeology.md","kind":"deprecated","code":1,"migration":"clms archaeology purge --session <v1-session> for cleanup"}
```

## migration from v1

```bash
# 1. find v1 sessions
clms --format ai context | jq -r '.[] | select(.agent=="archaeology") | .session' | sort -u

# 2. purge each
clms archaeology purge --session backfill-<v1-ts>

# 3. re-run with v2
clms archaeology suggest --max 10 > candidates.json
# orchestrate debate (pi-subagents recipe above)
clms archaeology commit --from-plan survivors.json --keep 8

# 4. claims land as pending. verify when ready:
clms verify <id> --method code-test --cmd "<suggested_evidence cmd>" ...
```

## test plan

- empty repo → 0 candidates, exit 0, message "no stake signals found"
- repo with 5 commits, no annotations, no tests → **0 candidates** (correct!
  commits alone are not signals in v2)
- repo with 3 `// clms-claim:` annotations → 3 candidates
- repo with 50 `// clms-claim:` annotations → exactly 10 candidates, ranked
- judge returns 0 survivors → commit succeeds, writes 0 claims, exit 0
- commit with `keep:true` row missing `debate.judge` → exits 1
- commit with `count(keep:true)=12, --keep 8` → exits 1
- re-running suggest+commit with same survivors.json → idempotent (skips on id collision)
- purge `--session <s>` removes only matching claims, leaves others intact
- dogfood on the clms repo: add 5 `// clms-claim:` annotations to src/, expect
  ≤5 candidates, run debate, expect ≤K survivors, all claim-shaped

## v3 deferred

- **readme-assertion** — prose-mining for "always X", "guarantees Y", etc.
  excluded from v2 because regex-on-marketing-copy is a noise generator
  (oracle's call). reconsider after dogfood data shows whether `clms-claim`
  annotation is sufficient.
- **docstring-invariant** — same rationale; defer.
- **clms archaeology refresh** — re-harvest, identify previously-committed
  claims whose source signal disappeared (test deleted, annotation removed).
  flags drift candidates for review.
- **multi-language `clms-claim:` extraction** — v2 ships rust+python comment
  syntaxes. add javascript/typescript/go variants based on demand.

## what this tool is honest about

- archaeology never produces verified claims. it produces *candidates* that
  survive an adversarial cut. promotion is the user's job via `clms verify`.
- the harvester is rule-based and intent-driven — it harvests what the
  codebase explicitly declares (`// clms-claim:`, test names, type marks),
  not what an llm guesses might be implicit.
- the debate is opt-in (the user runs the orchestrator) and overridable
  (transcripts are stored, judge rationales editable in survivors.json
  before commit).
- bounded-N is a feature, not a limitation. if the cap binds, the right
  move is tightening signals or adding `clms-claim` annotations to the
  codebase, NOT raising N.
- archaeology produces zero candidates on most fresh repos. that is correct.
  empty output is not a failure mode; it's a correct read of "no
  archaeology yield available here."
