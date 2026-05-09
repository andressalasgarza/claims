# claims

append-only ledger of falsifiable claims. agent-optimized. tamper-evident.

built for ai-agent-driven research loops where you need a deterministic trail
of what was believed, what was verified, what was refuted, and how it was all
proven. unlike git history or markdown notes, claims enforces:

- every verified claim must include reproducible evidence (no soft warnings, hard exit-1 on missing fields)
- every claim has a confidence tier auto-derived from its evidence method
- refutes/depends-on edges form a dag; refuting a claim cascades suspect-flags to dependents
- claim files are content-hashed for tamper-evidence

## install

```bash
cd /path/to/claims
cargo build --release
ln -s $(pwd)/target/release/clms ~/.local/bin/clms  # or wherever
```

for the `clms archaeology` workflow, also install the bundled
pi-subagents (`clms.judge`, `clms.proposer`) to user scope so any
orchestrator can spawn them regardless of cwd:

```bash
clms install-agents          # writes to ~/.pi/agent/agents/clms/
clms install-agents --force  # overwrite if you've upgraded clms
clms install-agents --dry-run  # print what would be written, touch nothing
```

verify:

```bash
# from any directory
subagent({ action: "list" })  # expect clms.judge + clms.proposer
```

## quickstart

```bash
cd /path/to/your/project
clms add "polymarket lags binance by ~300ms during BTC moves" --tag market:btc
# → #1

clms verify 1 \
  --method stat-test \
  --ref ./runs/lag_test.json \
  --test-type ks --p-value 0.003 --sample-size 4821 \
  --data-source real \
  --note "ks-test, 1 week of captured ticks"
# → #1 [verified · empirical]

clms add "we can arb this lag with fast router" --depends-on 1 --tag strategy:arb
# → #2

clms verify 2 --method integration-test --ref ./router_bench.sh --exit-code 0 \
  --target https://api.binance.com --cmd "bash ./router_bench.sh"
# → #2 [verified · empirical]

# later, you find the original test had lookahead bias:
clms add "lag is actually ~500ms not 300ms" --tag market:btc
clms verify 3 --method stat-test --ref ./runs/lag_v2.json \
  --test-type ks --p-value 0.001 --sample-size 9000 --data-source real

clms refute 1 --by 3 --reason "lookahead bias in v1 test" --cascade
# → #1 refuted, #2 auto-flagged suspect (it depended on #1)

clms suspect    # what now needs re-verification
clms timeline   # full history
clms context    # compact verified-only digest for agent context-stuffing
```

## states

| state | meaning |
|---|---|
| `pending` | logged, no evidence yet |
| `verified` | evidence attached, passed validation |
| `refuted` | proven wrong, points to replacement claim |
| `unverifiable` | explicitly cannot be tested (subjective, future-dependent); honesty bucket |
| `suspect` | a claim it depended on was refuted; needs re-verification |

## the falsifiability contract

every evidence method's data must come from a source the author does not
fully control. if the author picks both the input AND the expected output,
the "test" is a tautology check, not evidence. clms refuses these methods
at parse time:

| refused method | why |
|---|---|
| `unit-test` | confirmatory by construction. you wrote both the input and the assertion; the test cannot disagree with you. use `prop-test`, `integration-test`, `replay-test`, or `observed`. |
| `code-test` | removed in schema 1.1. ambiguous about which falsification surface applied. pick one of `prop-test` \| `integration-test` \| `replay-test`. |
| `sim-test` | running stats on synthetic data only proves your simulator behaves like itself (circular w/ the assumptions you're testing). use `stat-test --data-source=real`, or file as `derived`. |

if you cannot articulate a real falsification surface, the claim does not
belong in this ledger. clms is for falsifiable findings, not code
self-tests. that's what your test runner is for.

## confidence tiers (auto-derived from evidence)

| tier | comes from | example |
|---|---|---|
| `empirical` | `prop-test`, `integration-test`, `replay-test`, `stat-test` | counterexample-finding fuzz, real api probe, backtest on captured data, ks-test on real samples |
| `observed` | `observed` | tx hash, log line, captured response |
| `documented` | `documented` | official primary source + exact quote |
| `derived` | `derived` | inference from at least 2 other claims |

confidence is always max-tier across attached evidence. you cannot set it manually.

## evidence: hard requirements (no soft warnings)

each method requires specific fields. missing any → exit 1, no claim is written.

| method | required flags | falsification surface |
|---|---|---|
| `prop-test` | `--ref` `--exit-code` `--cmd` | randomized input generator (proptest/quickcheck/fuzz) |
| `integration-test` | `--ref` `--exit-code` `--target` `--cmd` | the real external system at `--target` |
| `replay-test` | `--ref` `--exit-code` `--dataset` `--cmd` | frozen real-world capture at `--dataset` |
| `stat-test` | `--ref` `--test-type` `--p-value` `--sample-size` `--data-source` | real \| live samples (simulated refused) |
| `observed` | `--ref` | a captured artifact |
| `documented` | `--ref` `--quote "<exact text>"` | primary-source document |
| `derived` | `--from <id>` `--from <id>` (min 2) | upstream claims (cascade on refute) |

local file refs (and `--dataset` on replay-test) are content-hashed at write
time. tampering is detectable. `--data-source=simulated` is rejected at
parse time.

## edges

each claim stores its own outgoing edges. reverse lookups via sqlite index.

| edge | meaning | side effect |
|---|---|---|
| `depends_on` | "i'm only true if X is true" | if X is refuted, i become suspect (with `--cascade`) |
| `tests` | "i was created to evaluate X" | neutral, outcome decides |
| `supports` | "independent evidence for X" | X's confidence may bump |
| `refines` | "X holds, but only under conditions Y" | X stays verified, gets a qualifier |
| `refutes` | "X is wrong" | X → refuted |

## storage

```
.claims/
├── 000001.json     ← canonical source of truth (content-hashed)
├── 000002.json
├── ...
└── index.db        ← sqlite, rebuilt anytime via `clms reindex`
```

claim files are immutable in spirit. mutations (verify, refute) update the
file in place but the content-hash + git history give tamper evidence. for
total immutability, commit `.claims/` to git after every write.

## ids

every claim has both:
- `ulid` (e.g. `01HXYZ4K7P9NQXM...`) — globally unique, time-sortable, used internally
- `seq` (e.g. `42`) — project-local monotonic int, human-friendly

cli accepts either: `clms show 42` and `clms show 01HXYZ...` both work.

## env

| var | effect |
|---|---|
| `CLAIMS_DIR` | override project root (default: cwd) |
| `CLAIMS_FORMAT` | default output format (`default` \| `human` \| `ai`) |
| `CLAIMS_AGENT` | auto-stamp every write with this agent name |
| `CLAIMS_SESSION` | auto-stamp every write with this session id |

## agent integration

two discovery commands designed to be run once at session start and cached:

```bash
clms --format ai schema   # machine-readable: requirement matrix per method,
                          # enum values, error-envelope shape, env vars
clms help-all             # human-readable: top-level + every subcommand's
                          # long help including examples and drift behavior
```

paste either into your agent's system context and it knows the whole cli
without trial-and-error. per-command help also works:

```bash
clms verify --help    # method-specific required fields, examples, drift behavior
clms refute --help    # --cascade semantics
clms rerun  --help    # when rerun is meaningful
```

### error envelopes under --format ai

when `--format ai` (or `CLAIMS_FORMAT=ai`) is set, errors emit a single-line
json object on **stderr** so the same parser handles both happy-path and
failure paths:

```bash
$ clms --format ai add
# stderr: {"clap_kind":"MissingRequiredArgument","code":2,"error":"...","field":"<TEXT>","kind":"clap"}
# exit 2

$ clms --format ai show 999
# stderr: {"code":1,"error":"claim #999 not found ...","kind":"runtime"}
# exit 1
```

shape: `{ error, kind: "clap"|"runtime", code: 1|2, clap_kind?, field? }`.
stdout still emits clean json on success.

run `clms --format ai schema` for the canonical envelope spec.

put this in your agent's system prompt:

> at session start, fetch `clms --format ai schema` once and cache it — it
> tells you exactly which fields each evidence method requires.
>
> before writing a claim, list every existing claim that must hold for yours
> to be true. pass each as `--depends-on <seq>`. if none → empty.
>
> never mark a claim verified without producing a reproducible artifact. use
> the appropriate `--method` and provide all required fields. the cli will
> reject incomplete evidence with exit 1 — read the error (or its json
> envelope on stderr under `--format ai`), do not retry blindly.
>
> always pass `--format ai` for json output. set `CLAIMS_AGENT=<your-name>`
> and `CLAIMS_SESSION=<run-id>` env vars so every write is auto-stamped.
>
> use `clms context --format ai` at session start to load known truth.
> use `clms suspect` to find claims that need re-verification.
> use `clms diff-evidence <id>` to inspect how a claim's support has evolved.

## commands

```
clms add <text> [--tag T] [--depends-on N] [--tests N] [--unverifiable]
              [--git-sha S] [--created-at RFC3339]   # historical-stamp overrides
clms verify <id> --method M --ref R [method-specific fields]
clms refute <id> --by <new_id> --reason "..." [--cascade]
clms show <id>
clms timeline [--tag T] [--exclude-agent A]
clms context  [--tag T] [--exclude-agent A]
clms suspect            [--exclude-agent A]
clms rerun <id> [--acknowledge-drift]
clms diff-evidence <id>
clms reindex
clms archaeology suggest [--max N] [--source K] [-o PATH]
clms archaeology commit  --from-plan PATH [--keep K]
clms archaeology purge   --session STAMP [--agent A]
clms install-agents          [--force] [--dry-run]
clms schema [methods]       # machine-readable schema (`methods` subtarget = just the requirement matrix; --format ai for json)
clms help-all               # every subcommand's long help in one dump
```

## archaeology (v2)

`clms archaeology` is a **candidacy engine, not a verification engine.** it
harvests claim-shaped signals from your codebase, runs them through an
adversarial debate, and writes survivors as `state: pending` claims that
you promote later via `clms verify`.

full design: docs/archaeology.md. tl;dr:

- v2.0 ships ONE signal kind (`clms-claim-annotation`) with TWO intent surfaces:
  - **human**: `// clms-claim:` / `# clms-claim:` in source code
  - **agent**: `.archaeology/proposals.json` (written by `.pi/agents/clms-proposer.md`)
- output is **bounded** at `--max=10` (ceiling 50). adding more sources
  doesn't add slots, they compete for the existing ones.
- archaeology **never auto-verifies.** every committed claim is `pending`
  with `evidence: []`. promotion is `clms verify`'s job, not archaeology's.
- debate phase is orchestrator-agnostic. pi-subagents reference impl uses
  the `clms.judge` agent (`.pi/agents/clms-judge.md`) with drop-as-default.

### usage

```rust
// clms-claim: ledger writes are append-only under concurrent fsync
// clms-evidence: method=prop-test cmd="cargo test --release ledger_append_props"
fn append_to_ledger(...) { ... }
```

```bash
# 0. (cold-start, optional): spawn clms.proposer to seed proposals.json
#    NO chat review; the judge phase is the discrimination gate

# 1. harvest candidates (reads source annotations + proposals.json if present)
clms archaeology suggest -o candidates.json

# 2. spawn clms.judge to debate -> writes survivors.json

# 3. ingest survivors as pending claims
clms archaeology commit --from-plan survivors.json --keep 8

# 4. promote each pending claim to verified when you actually run the test
#    pick the method that matches the falsification surface:
clms verify <id> --method prop-test --ref <path> --exit-code 0 --cmd "..."
# or integration-test --target ... / replay-test --dataset ... / stat-test --data-source real
```

### orchestration recipe (pi-subagents)

this is the full no-human-in-the-loop pipeline. the parent agent spawns
specialized children for the generative/discriminative steps; clms
handles the deterministic harvest + commit phases.

**important: the parent agent must DELEGATE to `clms.judge`, not
inline-judge.** the judge is a fresh-context, drop-by-default discriminator
whose verdict is unbiased by the parent's prior beliefs. an agent grading
its own work has obvious bias and was the original UX failure that
motivated this design.

```typescript
// step 0 (cold-start only) — proposer reads code, writes proposals.json
subagent({
  agent: "clms.proposer",
  task: `read the codebase under ${PROJECT_DIR}, identify load-bearing
    invariants, write up to 10 proposals to ${PROJECT_DIR}/.archaeology/proposals.json.
    do not modify any source file. return: "wrote N proposals".`,
})

// step 1 — harvester (deterministic, no agent needed)
// shell: clms archaeology suggest -o candidates.json

// step 2 — judge spawned by orchestrator (NOT inline-judged)
subagent({
  agent: "clms.judge",
  task: `apply drop-by-default judgement.
    INPUT: ${PROJECT_DIR}/candidates.json
    OUTPUT: ${PROJECT_DIR}/survivors.json
    return: "wrote N survivors, M cuts".`,
})

// step 3 — commit (deterministic, no agent needed)
// shell: clms archaeology commit --from-plan survivors.json
```

both agents are installed at user scope by `clms install-agents`. they
declare `inheritProjectContext: false` and `inheritSkills: false` so they
run against the explicit task input only, not parent conversation drift.

### filtering backfill from live context

```bash
clms context --exclude-agent archaeology   # only real-time claims
clms timeline --exclude-agent archaeology
clms suspect  --exclude-agent archaeology
```

### cleanup

remove a backfill session entirely (e.g. aborted run, v1 spew):

```bash
clms archaeology purge --session backfill-<rfc3339-ts>
```

### what archaeology cannot recover

- claims that died as bad ideas before being committed (survivorship bias)
- empirical-tier confidence (we refuse to fake re-running historical tests)
- stake itself — archaeology surfaces signals; the debate phase decides what
  represents real stake worth tracking. that decision is irreducibly
  intentional.

### v1 removed

the v1 git-and-mb-transcribe behavior is gone. it was a category error — it
wrote one verified claim per commit and one per mb entry, drowning the
ledger in unfalsifiable events. v1 sessions can be cleaned up with
`clms archaeology purge --session <stamp>`.

learn from our mistakes: docs/archaeology.md §"the lies v1 was telling."

global flags: `--format default|human|ai`, `--dir <path>`.

## non-goals

- not a task tracker (use marbles or todoist)
- not an adr tool (those track design decisions, clms tracks empirical findings)
- not a notebook (no narrative, no markdown bodies, only structured falsifiable statements)
