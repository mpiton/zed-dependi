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
pub(crate) fn sanitize_repo_url(_raw: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
}
