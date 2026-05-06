# clms archaeology — design

shadow-ledger reconstruction from existing repo signals. NOT a time machine —
an audit tool. produces a draft ledger of what was *implicitly believed* at each
point, derived from artifacts that already exist (git, mb).

## scope (locked)

- **git** is the universal source. always on.
- **mb** is optional, auto-detected via `.marbles/marbles.csv`.
- **no todoist, no jira, no plugin tier.** out of scope. users who want
  external trackers write their own one-off scripts that emit `clms add` calls;
  they are not first-class citizens in this binary.

## non-goals

- recovering claims that died as bad ideas before being committed (gone forever, survivorship bias is irreducible)
- empirical-tier evidence (would require re-running historical tests against historical environments — too rotten to be honest)
- semantic refute detection ("this refactor proved an earlier assumption wrong") — defer to v2
- merging across heterogeneous task trackers — only git and mb

## decisions

### (a) backfill isolation: agent-filter convention

backfilled claims live in the same `.claims/` dir as real-time entries.
distinguished by `agent: "archaeology"` and `session: "backfill-<rfc3339-ts>"`.

filtering is provided by `--exclude-agent archaeology` on `context`, `timeline`,
and `suspect` (delivered by m-f39f).

**rejected alternatives:**
- separate `.claims-draft/` dir → adds a second storage path, doubles `reindex`
  complexity, and requires a "promote" workflow nobody asked for
- reserved tag like `tag:archaeology` → tags are user-facing and shouldn't
  encode meta-provenance; agent stamp is the right channel

### (b) confidence cap: enforced by construction

archaeology only ever emits evidence with method `documented` or `observed`.
never `stat-test`, `code-test`, or `derived`. this is enforced by the type of
the evidence builder in `archaeology::emit`, not by a runtime check that could
be bypassed.

reasoning: re-running historical tests against today's environment is
dishonest. the ref hash you'd capture is today's hash of the historical file,
not the hash at the supposed verify time. drift detection downstream gets
permanently degraded. better to be transparent: "this is what the commit msg
*said* was true, here's the quote, that's all we know."

| source | method     | --ref           | --quote / payload          |
|--------|------------|-----------------|----------------------------|
| git    | documented | `git:<sha>`     | first line of commit msg   |
| mb     | documented | `mb:<id>`       | task title                 |

### (c) dedup: explicit id reference only

two entries are merged into one claim only when there's a hard cross-reference:

- commit message body contains literal `m-XXXX` → merge that commit's claim
  with the corresponding mb entry's claim
- otherwise: separate claims, even if text looks similar

**no fuzzy text matching, no llm-based similarity scoring.** if it ain't an
explicit id reference, it's two claims. the small redundancy cost is worth
avoiding spurious merges, which would silently corrupt the ledger.

### (d) refute detection: explicit reverts only

a commit refutes the prior claim when:

- commit message starts with `Revert "..."` (the standard `git revert` prefix), AND
- the message includes the reverted sha (also standard)

we look up that sha → find the corresponding archaeology claim → emit
`refute --by <new_seq> --reason "<commit msg>" --cascade`.

semantic refutes ("commit X proves earlier assumption Y wrong") are deferred.
agents will fail to detect them reliably from diffs alone.

## flags

```
clms archaeology [--since <sha>] [--no-mb] [--dry-run]

  --since <sha>   start from this sha (default: first commit)
  --no-mb         skip .marbles/marbles.csv even if present
  --dry-run       print planned writes as json, do not touch .claims/
```

global `--format ai` applies — under ai, both dry-run output and final report
are single-line json arrays per claim.

## stamping

every backfilled claim gets:

```
agent:      "archaeology"
session:    "backfill-<rfc3339-ts-of-archaeology-run>"
git_sha:    <historical sha being archaeologized, NOT current HEAD>
created_at: <historical commit timestamp, NOT now()>
updated_at: <same as created_at on first write>
```

historical `git_sha` and `created_at` are unblocked by m-f39f. without those,
every backfilled claim would lie about when and where it was made — which
defeats the point of an audit tool.

## flow

```
1. resolve --since (default to root commit)
2. walk git log --reverse --since <sha> → list of commits
3. if .marbles/ exists and !--no-mb: read marbles.csv → list of mb entries
4. build a sha → mb_id index from commit msgs (regex /m-[0-9a-f]+/)
5. for each commit:
     a. if msg starts with `Revert "..."` and contains a reverted sha:
          → look up reverted commit's archaeology claim
          → enqueue refute action
     b. else:
          → enqueue add action with documented evidence
6. for each mb entry not already linked to a commit:
     → enqueue add action with documented evidence
     → translate mb dep edges to --depends-on
7. for each linked (commit, mb) pair:
     → single add action, evidence references both
8. if --dry-run: serialize plan and exit
9. else: execute plan via store.write_claim, then verify_claim with
   the documented evidence, in chronological order so refute targets
   exist by the time refutes are processed
```

## test plan

minimum viable smoke tests (no formal test suite exists yet, hand-test):

- empty repo (only initial commit) → 1 claim emitted
- repo with 5 commits, no reverts → 5 claims, no refutes
- repo with explicit `git revert <sha>` → 2 claims, 1 refute edge
- repo with mb integration → claims merged on m-XXXX cross-ref
- `--dry-run` writes nothing to .claims/
- `--exclude-agent archaeology` on context/timeline/suspect hides all output
- run on the claims repo itself (dogfood) → produces a sane shadow ledger

## what this tool is honest about

- it cannot reconstruct what an agent *would have said* under real-time clms discipline (the "write falsifiable claim before knowing the answer" thing)
- it cannot confer empirical confidence retroactively, ever
- it cannot dedup across sources without explicit id refs, by design
- it produces a *plausible* shadow trail, not the *actual* epistemic trail

these constraints are surfaced in `clms archaeology --help` and the README so
users don't mistake the output for ground truth.
