use std::path::Path;

/// observed --ref must be one of:
///   - an existing local file (hashable; the H17 path will content_hash it)
///   - a URL with a recognized scheme (http/https/file/ftp/ssh/git)
///   - a content-address: sha256:HEX / blake3:HEX / sha1:HEX (any hex case)
///
/// random strings like "foo" or "my notes" are refused. without this check
/// observed degrades to "agent typed any string" and provides no auditable
/// surface for refutation.
pub(super) fn ref_shape_acceptable(s: &str) -> bool {
    if Path::new(s).exists() {
        return true;
    }
    let recognized_url_schemes = [
        "http://", "https://", "file://", "ftp://", "ftps://", "ssh://", "git://", "ws://", "wss://",
    ];
    if recognized_url_schemes.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // content-address: <hashname>:<hex>
    if let Some((scheme, hex)) = s.split_once(':') {
        if matches!(scheme, "sha256" | "sha1" | "sha512" | "blake3" | "md5")
            && !hex.is_empty()
            && hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            return true;
        }
    }
    false
}

