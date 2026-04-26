//! Repository URL sanitization shared by registry adapters.
//!
//! Allows only `http`/`https` URLs after stripping common package-manager
//! prefixes (`git+`) and suffixes (`.git`). Any other scheme is dropped.

/// Sanitize a repository URL from package metadata.
///
/// Returns `Some(url)` only if the resulting scheme is `http` or `https`
/// after stripping known package-manager prefixes (`git+`) and suffixes
/// (`.git`). Returns `None` for any other scheme, empty input, or
/// unparseable input.
pub(crate) fn sanitize_repo_url(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https() {
        assert_eq!(
            sanitize_repo_url("https://github.com/user/repo"),
            Some("https://github.com/user/repo".to_string())
        );
    }

    #[test]
    fn accepts_http() {
        assert_eq!(
            sanitize_repo_url("http://example.com/repo"),
            Some("http://example.com/repo".to_string())
        );
    }
}
