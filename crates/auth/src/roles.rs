//! Upstream claims to kampus roles (design §7). Each rule adds its role
//! when the claim equals its value, or is an array containing it; the roles
//! are the union of every matching rule. `admin` is never mapped (config
//! validation refuses it too).

use std::collections::BTreeSet;

use kampus_core::config::RoleMapping;
use kampus_core::contracts::Role;
use serde_json::Value;

/// The roles the validated ID-token `claims` earn under `rules`.
pub(crate) fn map(claims: &Value, rules: &[RoleMapping]) -> BTreeSet<Role> {
    rules
        .iter()
        .filter(|rule| rule.role != Role::Admin && matches(claims, rule))
        .map(|rule| rule.role)
        .collect()
}

fn matches(claims: &Value, rule: &RoleMapping) -> bool {
    // A top-level claim whose name itself contains dots (namespaced URL
    // claims) wins over the dotted-path reading.
    let value = claims.get(&rule.claim).or_else(|| {
        rule.claim
            .split('.')
            .try_fold(claims, |value, key| value.get(key))
    });
    match value {
        Some(Value::String(s)) => *s == rule.value,
        Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(&rule.value)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule(claim: &str, value: &str, role: Role) -> RoleMapping {
        RoleMapping {
            claim: claim.to_owned(),
            value: value.to_owned(),
            role,
        }
    }

    #[test]
    fn string_array_nested_and_union() {
        let rules = [
            rule("eduperson_affiliation", "student", Role::Student),
            rule("eduperson_affiliation", "staff", Role::Staff),
            rule("org.kampus.groups", "faculty", Role::Faculty),
            rule("https://idp.example/roles", "staff", Role::Staff),
        ];
        let single = json!({ "eduperson_affiliation": "student" });
        assert_eq!(map(&single, &rules), [Role::Student].into());
        let array = json!({ "eduperson_affiliation": ["member", "student", "staff"] });
        assert_eq!(map(&array, &rules), [Role::Student, Role::Staff].into());
        let nested = json!({ "org": { "kampus": { "groups": ["faculty"] } } });
        assert_eq!(map(&nested, &rules), [Role::Faculty].into());
        let namespaced = json!({ "https://idp.example/roles": ["staff"] });
        assert_eq!(map(&namespaced, &rules), [Role::Staff].into());
    }

    #[test]
    fn no_match_gives_no_role() {
        let rules = [rule("eduperson_affiliation", "student", Role::Student)];
        for claims in [
            json!({}),
            json!({ "eduperson_affiliation": "alum" }),
            json!({ "eduperson_affiliation": "Student" }),
            json!({ "eduperson_affiliation": { "student": true } }),
            json!({ "eduperson_affiliation": [["student"]] }),
            json!({ "roles": ["student"] }),
        ] {
            assert!(map(&claims, &rules).is_empty(), "{claims}");
        }
    }

    #[test]
    fn admin_is_never_mapped() {
        let rules = [rule("groups", "it", Role::Admin)];
        assert!(map(&json!({ "groups": ["it"] }), &rules).is_empty());
    }
}
