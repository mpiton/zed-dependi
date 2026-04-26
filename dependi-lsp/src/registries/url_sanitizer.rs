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
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("git+").unwrap_or(trimmed);

    let normalized = if let Some(rest) = stripped.strip_prefix("git://") {
        format!("https://{rest}")
    } else {
        stripped.to_string()
    };

    let mut parsed = url::Url::parse(&normalized).ok()?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let path = parsed.path().to_string();
    if let Some(without_git) = path.strip_suffix(".git") {
        parsed.set_path(without_git);
    }

    Some(parsed.to_string())
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

    #[test]
    fn strips_git_plus_https() {
        assert_eq!(
            sanitize_repo_url("git+https://github.com/user/repo.git"),
            Some("https://github.com/user/repo".to_string())
        );
    }

    #[test]
    fn strips_git_plus_http() {
        assert_eq!(
            sanitize_repo_url("git+http://example.com/repo.git"),
            Some("http://example.com/repo".to_string())
        );
    }

    #[test]
    fn legacy_git_protocol_converted_to_https() {
        assert_eq!(
            sanitize_repo_url("git://github.com/user/repo"),
            Some("https://github.com/user/repo".to_string())
        );
    }

    #[test]
    fn legacy_git_plus_git_protocol_converted_to_https() {
        assert_eq!(
            sanitize_repo_url("git+git://github.com/user/repo.git"),
            Some("https://github.com/user/repo".to_string())
        );
    }
}
