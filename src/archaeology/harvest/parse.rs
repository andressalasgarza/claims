use super::super::SuggestedEvidence;

pub(crate) fn parse_evidence_directive(raw: &str) -> SuggestedEvidence {
    let mut method = "prop-test".to_string();
    let mut cmd: Option<String> = None;
    let mut r#ref: Option<String> = None;
    let trimmed = raw.trim();
    for token in split_kv(trimmed) {
        if let Some((k, v)) = token.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "method" => method = v,
                "cmd" => cmd = Some(v),
                "ref" => r#ref = Some(v),
                _ => {}
            }
        }
    }
    SuggestedEvidence {
        method,
        cmd,
        r#ref,
        note: "advisory only; not run by archaeology. promotion via `clms verify`".into(),
    }
}

fn split_kv(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        if c == '"' {
            in_q = !in_q;
            cur.push(c);
        } else if c.is_whitespace() && !in_q {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
