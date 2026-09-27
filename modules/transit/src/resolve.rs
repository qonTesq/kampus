//! Case-insensitive resolution of configured stop and route aliases.

use kampus_core::config::{TransitRoute, TransitStop};
use kampus_core::contracts::ToolError;
use kampus_core::output::clean_text;

/// At most this many choices are offered for an unknown or ambiguous name.
const MAX_CHOICES: usize = 5;

/// Lowercase, drop periods and collapse whitespace: `Cecil B. Moore` and
/// `cecil  b moore` compare equal.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .replace('.', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn matches(input: &str, id: &str, name: Option<&str>, aliases: &[String]) -> bool {
    let key = normalize(input);
    normalize(id) == key
        || name.is_some_and(|n| normalize(n) == key)
        || aliases.iter().any(|a| normalize(a) == key)
}

/// Keeps five choices well inside the 500-character error message cap.
const CHOICE_MAX_CHARS: usize = 60;

fn choices<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut list: Vec<String> = names
        .take(MAX_CHOICES + 1)
        .map(|n| clean_text(n, CHOICE_MAX_CHARS))
        .collect();
    let more = list.len() > MAX_CHOICES;
    list.truncate(MAX_CHOICES);
    let mut out = list.join("; ");
    if more {
        out.push_str("; …");
    }
    out
}

/// One configured stop by ID, name or alias. Unknown and ambiguous names
/// are rejected with bounded configured choices; nothing is guessed.
pub(crate) fn stop<'a>(
    stops: &'a [TransitStop],
    input: &str,
) -> Result<&'a TransitStop, ToolError> {
    // An exact canonical ID always wins over aliases.
    if let Some(s) = stops.iter().find(|s| s.id == input.trim()) {
        return Ok(s);
    }
    let found: Vec<&TransitStop> = stops
        .iter()
        .filter(|s| matches(input, &s.id, Some(&s.name), &s.aliases))
        .collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(ToolError::invalid_arguments(format!(
            "Unknown stop. Configured stops: {}.",
            choices(stops.iter().map(|s| s.name.as_str()))
        ))),
        many => Err(ToolError::invalid_arguments(format!(
            "That name matches several stops; ask again with one of: {}.",
            choices(many.iter().map(|s| s.name.as_str()))
        ))),
    }
}

/// One configured route by ID or alias.
pub(crate) fn route<'a>(
    routes: &'a [TransitRoute],
    input: &str,
) -> Result<&'a TransitRoute, ToolError> {
    if let Some(r) = routes.iter().find(|r| r.id == input.trim()) {
        return Ok(r);
    }
    let found: Vec<&TransitRoute> = routes
        .iter()
        .filter(|r| matches(input, &r.id, None, &r.aliases))
        .collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(ToolError::invalid_arguments(format!(
            "Unknown route. Configured routes: {}.",
            choices(routes.iter().map(|r| r.id.as_str()))
        ))),
        many => Err(ToolError::invalid_arguments(format!(
            "That name matches several routes; ask again with one of: {}.",
            choices(many.iter().map(|r| r.id.as_str()))
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop_cfg(id: &str, name: &str, aliases: &[&str]) -> TransitStop {
        TransitStop {
            id: id.into(),
            name: name.into(),
            provider: "p".into(),
            aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    #[test]
    fn normalizes_case_periods_and_spaces() {
        let stops = [stop_cfg("1", "Cecil B. Moore", &[])];
        assert_eq!(stop(&stops, "  cecil b  moore ").unwrap().id, "1");
    }

    #[test]
    fn ambiguous_choices_are_bounded() {
        let stops: Vec<_> = (0..8)
            .map(|i| stop_cfg(&i.to_string(), &format!("Stop {i}"), &["main"]))
            .collect();
        let err = stop(&stops, "MAIN").unwrap_err();
        assert_eq!(
            err.code,
            kampus_core::contracts::ErrorCode::InvalidArguments
        );
        assert!(err.message.contains("Stop 4; …"));
        assert!(!err.message.contains("Stop 5"));
    }
}
