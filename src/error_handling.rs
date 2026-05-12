//! json error envelope for `--format ai` mode + early-flag detection so we
//! can emit machine-readable errors even when clap rejects the args before
//! `Cli::parse()` can populate `Cli::format`. only `main()` consumes these.

/// peek at argv (and `CLAIMS_FORMAT`) for the `--format` value before clap
/// has parsed anything. needed because clap errors fire before we have a
/// populated `Cli` struct to read `cli.format` from.
pub(crate) fn detect_early_format() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--format" {
            if let Some(next) = args.get(i + 1) {
                return Some(next.clone());
            }
        }
        if let Some(rest) = a.strip_prefix("--format=") {
            return Some(rest.to_string());
        }
        i += 1;
    }
    std::env::var("CLAIMS_FORMAT").ok().filter(|s| !s.is_empty())
}

pub(crate) fn is_ai_format(s: &Option<String>) -> bool {
    s.as_deref()
        .map(|x| x.eq_ignore_ascii_case("ai"))
        .unwrap_or(false)
}

/// emit the standard json error envelope on stderr. all `--format ai`
/// failures funnel through this so agents get a stable shape:
/// `{error, kind, code, ...extras}`.
pub(crate) fn emit_json_error(err: &str, kind: &str, code: i32, extras: serde_json::Value) {
    let mut env = serde_json::json!({
        "error": err,
        "kind": kind,
        "code": code,
    });
    if let serde_json::Value::Object(map) = extras {
        if let serde_json::Value::Object(out) = &mut env {
            for (k, v) in map {
                out.insert(k, v);
            }
        }
    }
    eprintln!("{}", env);
}

/// translate a `clap::Error` into a (msg, extras) tuple for `emit_json_error`.
/// extracts the offending field name when present so agents know which arg
/// to fix.
pub(crate) fn clap_err_extras(e: &clap::Error) -> (String, serde_json::Value) {
    use clap::error::ContextKind;
    let mut field: Option<String> = None;
    for (ck, cv) in e.context() {
        if matches!(ck, ContextKind::InvalidArg) {
            field = Some(cv.to_string());
            break;
        }
    }
    let kind_dbg = format!("{:?}", e.kind());
    let msg = e
        .to_string()
        .lines()
        .next()
        .unwrap_or("clap parse error")
        .trim_start_matches("error: ")
        .to_string();
    let mut extras = serde_json::Map::new();
    extras.insert("clap_kind".into(), serde_json::Value::String(kind_dbg));
    if let Some(f) = field {
        extras.insert("field".into(), serde_json::Value::String(f));
    }
    (msg, serde_json::Value::Object(extras))
}
