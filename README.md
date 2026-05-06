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

## quickstart

```bash
cd /path/to/your/project
clms add "polymarket lags binance by ~300ms during BTC moves" --tag market:btc
# → #1

clms verify 1 \
  --method stat-test \
  --ref ./runs/lag_test.json \
  --test-type ks --p-value 0.003 --sample-size 4821 \
  --note "ks-test, 1 week sample"
# → #1 [verified · empirical]

clms add "we can arb this lag with fast router" --depends-on 1 --tag strategy:arb
# → #2

clms verify 2 --method code-test --ref ./router_bench.sh --exit-code 0
# → #2 [verified · empirical]

# later, you find the original test had lookahead bias:
clms add "lag is actually ~500ms not 300ms" --tag market:btc
clms verify 3 --method stat-test --ref ./runs/lag_v2.json \
  --test-type ks --p-value 0.001 --sample-size 9000

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

## confidence tiers (auto-derived from evidence)

| tier | comes from | example |
|---|---|---|
| `empirical` | `stat-test` or `code-test` | p<0.01 ks-test, curl returns expected, exit 0 |
| `observed` | `observed` | tx hash, log line, captured response |
| `documented` | `documented` | official primary source + exact quote |
| `derived` | `derived` | inference from at least 2 other claims |

confidence is always max-tier across attached evidence. you cannot set it manually.

## evidence: hard requirements (no soft warnings)

each method requires specific fields. missing any → exit 1, no claim is written.

| method | required flags |
|---|---|
| `stat-test` | `--ref` `--test-type` `--p-value` `--sample-size` |
| `code-test` | `--ref` `--exit-code` |
| `observed` | `--ref` |
| `documented` | `--ref` `--quote "<exact text>"` |
| `derived` | `--from <id>` `--from <id>` (min 2) |

local file refs are content-hashed at write time. tampering is detectable.

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

for full command reference in one shot (top-level help + every subcommand's
long help including required-fields tables and examples):

```bash
clms help-all
```

paste that into your agent's system context once and it knows the whole cli.
per-command help also works:

```bash
clms verify --help    # shows method-specific required fields, examples, drift behavior
clms refute --help    # shows --cascade semantics
clms rerun  --help    # shows when rerun is meaningful
```

put this in your agent's system prompt:

> before writing a claim, list every existing claim that must hold for yours
> to be true. pass each as `--depends-on <seq>`. if none → empty.
>
> never mark a claim verified without producing a reproducible artifact. use
> the appropriate `--method` and provide all required fields. the cli will
> reject incomplete evidence with exit 1 — read the error, do not retry blindly.
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
clms verify <id> --method M --ref R [method-specific fields]
clms refute <id> --by <new_id> --reason "..." [--cascade]
clms show <id>
clms timeline [--tag T]
clms context  [--tag T]
clms suspect
clms reindex
```

global flags: `--format default|human|ai`, `--dir <path>`.

## non-goals

- not a task tracker (use marbles or todoist)
- not an adr tool (those track design decisions, clms tracks empirical findings)
- not a notebook (no narrative, no markdown bodies, only structured falsifiable statements)
