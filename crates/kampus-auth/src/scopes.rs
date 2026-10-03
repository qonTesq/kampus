//! Scope strings (RFC 6749 section 3.3): space-delimited, order-insensitive,
//! case-sensitive tokens.

use std::collections::BTreeSet;

use kampus_core::config::is_scope_token;

/// The refresh-related scope; accepted, never placed in an access token.
pub(crate) const OFFLINE_ACCESS: &str = "offline_access";

/// Parse a scope parameter into a set. `None` if any token is malformed.
pub(crate) fn parse(scope: &str) -> Option<BTreeSet<String>> {
    scope
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(|s| is_scope_token(s).then(|| s.to_owned()))
        .collect()
}

/// The canonical string form: sorted, single spaces.
pub(crate) fn format(scopes: &BTreeSet<String>) -> String {
    scopes.iter().map(String::as_str).collect::<Vec<_>>().join(" ")
}

/// Requested scopes minus `offline_access`, if every one is grantable.
pub(crate) fn grantable(
    requested: &BTreeSet<String>,
    configured: &BTreeSet<String>,
) -> Option<BTreeSet<String>> {
    requested
        .iter()
        .filter(|s| *s != OFFLINE_ACCESS)
        .map(|s| configured.contains(s).then(|| s.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rules() {
        assert_eq!(
            parse("b a  a").unwrap(),
            ["a".to_owned(), "b".to_owned()].into()
        );
        assert_eq!(parse("").unwrap(), BTreeSet::new());
        assert!(parse("a\"b").is_none());
        assert!(parse("a\tb").is_none());
        let configured: BTreeSet<String> = ["rooms:book".to_owned()].into();
        assert_eq!(
            grantable(&parse("offline_access rooms:book").unwrap(), &configured).unwrap(),
            configured
        );
        assert!(grantable(&parse("admin").unwrap(), &configured).is_none());
    }
}
