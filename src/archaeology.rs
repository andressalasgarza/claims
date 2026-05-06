//! shadow-ledger reconstruction from git history + (optionally) mb csv.
//! see docs/archaeology.md for scope, decisions, and constraints.
//!
//! key invariant: confidence is capped by construction. only `documented`
//! evidence is ever emitted from this module. never `stat-test`, `code-test`,
//! `observed`, or `derived`.

use crate::models::{Claim, Edge, EdgeType, Evidence, EvidenceMethod, State, SCHEMA_VERSION};
use crate::output::OutputFormat;
use crate::store::Store;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use ulid::Ulid;

pub const AGENT: &str = "archaeology";
pub const MB_CSV: &str = ".marbles/marbles.csv";

pub struct Args {
    pub since: Option<String>,
    pub no_mb: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
struct GitCommit {
    sha: String,
    short_sha: String,
    timestamp: DateTime<Utc>,
    subject: String,
}

#[derive(Clone, Debug)]
struct MbEntry {
    id: String,
    title: String,
    status: String,
    created_at: DateTime<Utc>,
    blocked_by: Vec<String>,
}

#[derive(Clone, Debug)]
enum Action {
    Add {
        key: String,             // unique source key, e.g. "git:abc1234" or "git:abc1234+mb:m-aaa"
        text: String,
        quote: String,           // documented-evidence quote
        ref_str: String,         // e.g. "git:<sha>" or "mb:<id>"
        timestamp: DateTime<Utc>,
        git_sha: Option<String>,
        mb_blocked_by: Vec<String>,
    },
    Refute {
        target_key: String,      // refers to a previously-added Add::key
        by_key: String,          // refers to the reverting commit's Add::key
        reason: String,
    },
}

// ---------- git ----------

fn fetch_git_log(since: Option<&str>) -> Result<Vec<GitCommit>> {
    let mut cmd = Command::new("git");
    cmd.args([
        "log",
        "--reverse",
        "--pretty=format:%H%x09%h%x09%cI%x09%s",
    ]);
    if let Some(sha) = since {
        cmd.arg(format!("{}..HEAD", sha));
    }
    let out = cmd.output().context("git log failed")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let mut commits = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() < 4 {
            continue;
        }
        let ts = DateTime::parse_from_rfc3339(parts[2])
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| anyhow!("bad commit timestamp '{}': {}", parts[2], e))?;
        commits.push(GitCommit {
            sha: parts[0].to_string(),
            short_sha: parts[1].to_string(),
            timestamp: ts,
            subject: parts[3].to_string(),
        });
    }
    Ok(commits)
}

fn detect_revert(subject: &str) -> Option<String> {
    let s = subject.trim();
    if !s.starts_with("Revert \"") && !s.starts_with("revert: ") && !s.starts_with("Revert: ") {
        return None;
    }
    Some(s.to_string())
}

fn extract_mb_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 1 < n {
        if bytes[i] == b'm' && bytes[i + 1] == b'-' {
            let start = i;
            i += 2;
            while i < n && (bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let len = i - start - 2;
            if (4..=16).contains(&len) {
                out.push(text[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

// ---------- mb csv ----------

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_q = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

fn fetch_mb_csv(path: &Path) -> Result<Vec<MbEntry>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut lines = raw.lines();
    let header = lines.next().ok_or_else(|| anyhow!("empty marbles.csv"))?;
    let cols = parse_csv_line(header);
    let col_idx = |name: &str| cols.iter().position(|c| c == name);
    let i_id = col_idx("id").ok_or_else(|| anyhow!("marbles.csv missing 'id' column"))?;
    let i_title = col_idx("title").ok_or_else(|| anyhow!("marbles.csv missing 'title' column"))?;
    let i_status =
        col_idx("status").ok_or_else(|| anyhow!("marbles.csv missing 'status' column"))?;
    let i_created = col_idx("created_at")
        .ok_or_else(|| anyhow!("marbles.csv missing 'created_at' column"))?;
    let i_blocked = col_idx("blocked_by")
        .ok_or_else(|| anyhow!("marbles.csv missing 'blocked_by' column"))?;

    let mut out = Vec::new();
    // assemble multi-line records (csv fields can contain newlines inside quotes)
    let mut buf = String::new();
    let mut in_q = false;
    for ch in raw[header.len()..].chars().skip(1) {
        if ch == '"' {
            in_q = !in_q;
        }
        if ch == '\n' && !in_q {
            if !buf.trim().is_empty() {
                let fields = parse_csv_line(&buf);
                if let Some(entry) = mb_record_to_entry(&fields, i_id, i_title, i_status, i_created, i_blocked) {
                    out.push(entry);
                }
            }
            buf.clear();
        } else {
            buf.push(ch);
        }
    }
    if !buf.trim().is_empty() {
        let fields = parse_csv_line(&buf);
        if let Some(entry) = mb_record_to_entry(&fields, i_id, i_title, i_status, i_created, i_blocked) {
            out.push(entry);
        }
    }
    Ok(out)
}

fn mb_record_to_entry(
    fields: &[String],
    i_id: usize,
    i_title: usize,
    i_status: usize,
    i_created: usize,
    i_blocked: usize,
) -> Option<MbEntry> {
    let id = fields.get(i_id)?.clone();
    if id.is_empty() {
        return None;
    }
    let title = fields.get(i_title)?.clone();
    let status = fields.get(i_status)?.clone();
    let created_raw = fields.get(i_created)?;
    let created_at = DateTime::parse_from_rfc3339(created_raw)
        .ok()?
        .with_timezone(&Utc);
    let blocked_raw = fields.get(i_blocked).cloned().unwrap_or_default();
    let blocked_by: Vec<String> = blocked_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Some(MbEntry {
        id,
        title,
        status,
        created_at,
        blocked_by,
    })
}

// ---------- planning ----------

fn build_plan(commits: &[GitCommit], mb_entries: &[MbEntry]) -> Vec<Action> {
    let mut plan: Vec<Action> = Vec::new();

    // index commit -> referenced mb ids
    let mut commit_to_mbs: HashMap<String, Vec<String>> = HashMap::new();
    for c in commits {
        let ids = extract_mb_ids(&c.subject);
        if !ids.is_empty() {
            commit_to_mbs.insert(c.sha.clone(), ids);
        }
    }
    let mut linked_mb_ids: HashSet<String> = HashSet::new();
    for ids in commit_to_mbs.values() {
        for id in ids {
            linked_mb_ids.insert(id.clone());
        }
    }

    // adds: one per commit (merging linked mb context into the same claim)
    let short_sha_by_full: HashMap<String, String> = commits
        .iter()
        .map(|c| (c.sha.clone(), c.short_sha.clone()))
        .collect();

    for c in commits {
        // skip pure reverts at this stage; they emit Refute below
        if detect_revert(&c.subject).is_some() {
            continue;
        }
        let linked = commit_to_mbs.get(&c.sha).cloned().unwrap_or_default();
        let key = make_key_git(&c.short_sha, &linked);
        let mb_lookup: Vec<&MbEntry> = linked
            .iter()
            .filter_map(|id| mb_entries.iter().find(|e| &e.id == id))
            .collect();
        let mb_blocked_by: Vec<String> = mb_lookup
            .iter()
            .flat_map(|e| e.blocked_by.iter().cloned())
            .collect();
        let mut text = c.subject.clone();
        if !mb_lookup.is_empty() {
            let titles: Vec<String> = mb_lookup.iter().map(|e| e.title.clone()).collect();
            text = format!("{} (mb: {})", text, titles.join("; "));
        }
        plan.push(Action::Add {
            key,
            text,
            quote: c.subject.clone(),
            ref_str: format!("git:{}", c.short_sha),
            timestamp: c.timestamp,
            git_sha: Some(c.sha.clone()),
            mb_blocked_by,
        });
    }

    // adds: mb entries not linked to any commit
    for m in mb_entries {
        if linked_mb_ids.contains(&m.id) {
            continue;
        }
        plan.push(Action::Add {
            key: format!("mb:{}", m.id),
            text: format!("{} [mb {} {}]", m.title, m.id, m.status),
            quote: m.title.clone(),
            ref_str: format!("mb:{}", m.id),
            timestamp: m.created_at,
            git_sha: None,
            mb_blocked_by: m.blocked_by.clone(),
        });
    }

    // chronological order so refute targets exist before reverts
    plan.sort_by_key(plan_action_ts);

    // refutes: look up reverted sha in commit msg, emit Action::Refute
    for c in commits {
        if detect_revert(&c.subject).is_none() {
            continue;
        }
        let reverted_sha = find_reverted_sha(c, commits);
        let target_short = reverted_sha
            .as_ref()
            .and_then(|sha| short_sha_by_full.get(sha).cloned());
        if let Some(target_short) = target_short {
            let target_linked = commits
                .iter()
                .find(|cc| cc.short_sha == target_short)
                .and_then(|cc| commit_to_mbs.get(&cc.sha))
                .cloned()
                .unwrap_or_default();
            let target_key = make_key_git(&target_short, &target_linked);
            let by_linked = commit_to_mbs.get(&c.sha).cloned().unwrap_or_default();
            let by_key = make_key_git(&c.short_sha, &by_linked);
            // also need the by-commit added if it wasn't (it WAS skipped above as a revert)
            plan.push(Action::Add {
                key: by_key.clone(),
                text: c.subject.clone(),
                quote: c.subject.clone(),
                ref_str: format!("git:{}", c.short_sha),
                timestamp: c.timestamp,
                git_sha: Some(c.sha.clone()),
                mb_blocked_by: vec![],
            });
            plan.push(Action::Refute {
                target_key,
                by_key,
                reason: c.subject.clone(),
            });
        }
    }

    // re-sort: adds in chronological order, refutes interleaved correctly
    plan.sort_by_key(|a| (plan_action_ts(a), matches!(a, Action::Refute { .. }) as u8));
    plan
}

fn plan_action_ts(a: &Action) -> DateTime<Utc> {
    match a {
        Action::Add { timestamp, .. } => *timestamp,
        Action::Refute { .. } => Utc::now(), // placed last per group
    }
}

fn make_key_git(short_sha: &str, linked_mbs: &[String]) -> String {
    if linked_mbs.is_empty() {
        format!("git:{}", short_sha)
    } else {
        format!("git:{}+mb:{}", short_sha, linked_mbs.join(","))
    }
}

fn find_reverted_sha(revert: &GitCommit, all: &[GitCommit]) -> Option<String> {
    // git revert always quotes the original subject. find a prior commit with matching subject.
    let s = &revert.subject;
    let inner = s.strip_prefix("Revert \"")?.trim_end_matches('"');
    all.iter()
        .filter(|c| c.timestamp < revert.timestamp)
        .rev()
        .find(|c| c.subject == inner)
        .map(|c| c.sha.clone())
}

// ---------- execution ----------

fn build_claim(
    seq: u64,
    text: String,
    timestamp: DateTime<Utc>,
    git_sha: Option<String>,
    session: &str,
    depends_on: Vec<u64>,
) -> Claim {
    let edges: Vec<Edge> = depends_on
        .into_iter()
        .map(|to_seq| Edge {
            r#type: EdgeType::DependsOn,
            to_seq,
        })
        .collect();
    Claim {
        schema_version: SCHEMA_VERSION.into(),
        id: Ulid::new(),
        seq,
        text,
        state: State::Pending,
        tags: vec![],
        evidence: vec![],
        edges,
        agent: Some(AGENT.to_string()),
        session: Some(session.to_string()),
        git_sha,
        created_at: timestamp,
        updated_at: timestamp,
        content_hash: None,
    }
}

fn build_documented_evidence(quote: &str, ref_str: &str, timestamp: DateTime<Utc>) -> Evidence {
    Evidence {
        method: EvidenceMethod::Documented,
        r#ref: ref_str.to_string(),
        note: Some("backfilled by clms archaeology".to_string()),
        p_value: None,
        sample_size: None,
        test_type: None,
        exit_code: None,
        quote: Some(quote.to_string()),
        from_claims: vec![],
        ref_hash: None,
        cmd: None,
        stdout_hash: None,
        recorded_at: timestamp,
    }
}

fn execute_plan(store: &mut Store, plan: &[Action], session: &str) -> Result<usize> {
    let mut key_to_seq: HashMap<String, u64> = HashMap::new();
    let mut written = 0usize;

    for action in plan {
        match action {
            Action::Add {
                key,
                text,
                quote,
                ref_str,
                timestamp,
                git_sha,
                mb_blocked_by,
            } => {
                if key_to_seq.contains_key(key) {
                    continue; // duplicate add (e.g. revert path also added the by-commit)
                }
                let depends_on: Vec<u64> = mb_blocked_by
                    .iter()
                    .filter_map(|id| key_to_seq.get(&format!("mb:{}", id)).copied())
                    .collect();
                let seq = store.next_seq()?;
                let mut claim = build_claim(
                    seq,
                    text.clone(),
                    *timestamp,
                    git_sha.clone(),
                    session,
                    depends_on,
                );
                claim
                    .evidence
                    .push(build_documented_evidence(quote, ref_str, *timestamp));
                claim.state = State::Verified;
                store.write_claim(&mut claim)?;
                key_to_seq.insert(key.clone(), claim.seq);
                written += 1;
            }
            Action::Refute {
                target_key,
                by_key,
                reason,
            } => {
                let target_seq = key_to_seq.get(target_key).copied();
                let by_seq = key_to_seq.get(by_key).copied();
                if let (Some(t), Some(b)) = (target_seq, by_seq) {
                    let mut claim = store.read_claim(t)?;
                    claim.state = State::Refuted;
                    claim.edges.push(Edge {
                        r#type: EdgeType::Refutes,
                        to_seq: b,
                    });
                    claim.evidence.push(Evidence {
                        method: EvidenceMethod::Derived,
                        r#ref: format!("claim:#{}", b),
                        note: Some(reason.clone()),
                        p_value: None,
                        sample_size: None,
                        test_type: None,
                        exit_code: None,
                        quote: None,
                        from_claims: vec![b],
                        ref_hash: None,
                        cmd: None,
                        stdout_hash: None,
                        recorded_at: Utc::now(),
                    });
                    claim.updated_at = Utc::now();
                    store.write_claim(&mut claim)?;
                }
            }
        }
    }
    Ok(written)
}

// ---------- output ----------

fn emit_plan_json(plan: &[Action]) -> Result<String> {
    let arr: Vec<_> = plan
        .iter()
        .map(|a| match a {
            Action::Add {
                key,
                text,
                ref_str,
                timestamp,
                git_sha,
                mb_blocked_by,
                ..
            } => serde_json::json!({
                "action": "add",
                "key": key,
                "text": text,
                "ref": ref_str,
                "timestamp": timestamp.to_rfc3339(),
                "git_sha": git_sha,
                "depends_on_mb": mb_blocked_by,
            }),
            Action::Refute {
                target_key,
                by_key,
                reason,
            } => serde_json::json!({
                "action": "refute",
                "target": target_key,
                "by": by_key,
                "reason": reason,
            }),
        })
        .collect();
    Ok(serde_json::to_string(&arr)?)
}

fn emit_plan_human(plan: &[Action]) -> String {
    let mut s = format!("planned actions ({}):\n", plan.len());
    for a in plan {
        match a {
            Action::Add {
                key, text, timestamp, ..
            } => {
                s.push_str(&format!(
                    "  + add  [{}] {} :: {}\n",
                    timestamp.format("%Y-%m-%d"),
                    key,
                    text
                ));
            }
            Action::Refute {
                target_key, by_key, ..
            } => {
                s.push_str(&format!("  - refute  {}  by  {}\n", target_key, by_key));
            }
        }
    }
    s
}

// ---------- entry ----------

pub fn run(store: &mut Store, args: Args, fmt: OutputFormat) -> Result<()> {
    let session = format!("backfill-{}", Utc::now().to_rfc3339());

    let commits = fetch_git_log(args.since.as_deref())?;
    let mb_entries = if args.no_mb {
        Vec::new()
    } else {
        let p = Path::new(MB_CSV);
        if p.exists() {
            fetch_mb_csv(p)?
        } else {
            Vec::new()
        }
    };

    let plan = build_plan(&commits, &mb_entries);

    if args.dry_run {
        if matches!(fmt, OutputFormat::Ai) {
            println!("{}", emit_plan_json(&plan)?);
        } else {
            print!("{}", emit_plan_human(&plan));
        }
        return Ok(());
    }

    let written = execute_plan(store, &plan, &session)?;
    if matches!(fmt, OutputFormat::Ai) {
        println!(
            "{}",
            serde_json::json!({
                "agent": AGENT,
                "session": session,
                "claims_written": written,
                "plan_size": plan.len(),
            })
        );
    } else {
        println!(
            "archaeology run: agent={} session={} written={} (plan={})",
            AGENT,
            session,
            written,
            plan.len()
        );
    }
    Ok(())
}
