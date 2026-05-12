use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// is this --target a loopback or RFC1918 / link-local / unspecified address?
/// rejects: localhost, 127/8, 0/8, 10/8, 172.16/12, 192.168/16, 169.254/16,
/// ::1, fc00::/7, fe80::/10. matches by hostname *or* parsed IP literal so
/// both `https://localhost:8080/health` and `http://127.0.0.1` are caught.
/// extract the bare host from a url-shaped target: strip scheme, path,
/// :port suffix, and surrounding [ ] for ipv6 literals.
fn extract_host_from_target(target: &str) -> &str {
    let after_scheme = target.split_once("://").map(|(_, r)| r).unwrap_or(target);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = match host_port.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host_port,
    };
    host.trim_start_matches('[').trim_end_matches(']')
}

/// hard-coded local host literals. matched case-insensitively against the
/// extracted host. centralizing the list keeps target_is_local flat.
const LOCAL_HOST_LITERALS: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "",
    "::",
    "::1",
];

/// ipv4 loopback/private/link-local/unspecified/0.0.0.0-prefix check.
/// extracted so target_is_local stays flat.
fn ipv4_is_local(v4: Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.octets()[0] == 0
}

/// ipv6 loopback/unspecified/unique-local(fc00::/7)/link-local(fe80::/10).
fn ipv6_is_local(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    let s = v6.segments()[0];
    (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
}

pub fn target_is_local(target: &str) -> bool {
    let host = extract_host_from_target(target);
    let lowered = host.to_ascii_lowercase();
    if LOCAL_HOST_LITERALS.iter().any(|lit| *lit == lowered) {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => ipv4_is_local(v4),
        Ok(IpAddr::V6(v6)) => ipv6_is_local(v6),
        Err(_) => false,
    }
}

