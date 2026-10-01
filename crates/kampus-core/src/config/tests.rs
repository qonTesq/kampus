use std::collections::BTreeSet;

use super::*;
use crate::contracts::{DataClass, Role};

const TEMPLE_SCHOOL: &str = include_str!("../../../../schools/temple/school.yaml");
const TEMPLE_POLICY: &str = include_str!("../../../../schools/temple/policy.yaml");

const TOOLS: [&str; 7] = [
    "find_study_room",
    "book_study_room",
    "confirm_booking",
    "search_courses",
    "next_departures",
    "my_deadlines",
    "library_hours",
];

fn tools() -> BTreeSet<String> {
    TOOLS.iter().map(|t| (*t).to_owned()).collect()
}

fn env(sandbox_compiled: bool) -> Env<'static> {
    Env {
        has_var: &|_| true,
        sandbox_compiled,
    }
}

fn school() -> SchoolConfig {
    SchoolConfig::from_yaml(TEMPLE_SCHOOL).unwrap()
}

fn policy() -> PolicyConfig {
    PolicyConfig::from_yaml(TEMPLE_POLICY).unwrap()
}

/// Field names of the validation issues, or a panic if validation passed.
fn school_issues(s: &SchoolConfig, env: &Env<'_>) -> Vec<String> {
    match s.validate(env) {
        Err(ConfigError::Invalid(issues)) => issues.into_iter().map(|i| i.field).collect(),
        other => panic!("expected validation issues, got {other:?}"),
    }
}

fn policy_issues(p: &PolicyConfig, env: &Env<'_>) -> Vec<String> {
    match p.validate(&school(), &tools(), env) {
        Err(ConfigError::Invalid(issues)) => issues.into_iter().map(|i| i.field).collect(),
        other => panic!("expected validation issues, got {other:?}"),
    }
}

#[test]
fn temple_profile_is_valid() {
    let s = school();
    s.validate(&env(false)).unwrap();
    policy().validate(&s, &tools(), &env(false)).unwrap();
    assert_eq!(s.school.timezone, "America/New_York");
    assert_eq!(s.libcal.as_ref().unwrap().default_location, "8404");
    assert_eq!(s.banner.as_ref().unwrap().default_term, "202636");
}

#[test]
fn unknown_field_is_a_parse_error() {
    let yaml = TEMPLE_SCHOOL.replace(
        "  timezone: America/New_York",
        "  timezone: America/New_York\n  colour: red",
    );
    assert!(matches!(
        SchoolConfig::from_yaml(&yaml),
        Err(ConfigError::Parse { .. })
    ));
    let yaml = TEMPLE_POLICY.replace(
        "sensitive_enabled: false",
        "sensitive_enabled: false\nextra: 1",
    );
    assert!(matches!(
        PolicyConfig::from_yaml(&yaml),
        Err(ConfigError::Parse { .. })
    ));
}

#[test]
fn leftover_audience_alias_fails_with_a_field_diagnostic() {
    let yaml = TEMPLE_SCHOOL.replacen("auth:\n", "auth:\n  audience: kampus-temple\n", 1);
    match SchoolConfig::from_yaml(&yaml) {
        Err(ConfigError::Parse { message, .. }) => {
            assert!(message.contains("audience"), "{message}");
        }
        other => panic!("expected a parse error naming audience, got {other:?}"),
    }
}

#[test]
fn roles_claim_defaults_to_roles_and_accepts_a_dotted_path() {
    assert_eq!(school().auth.roles_claim, "roles");
    let yaml = TEMPLE_SCHOOL.replacen("auth:\n", "auth:\n  roles_claim: entitlements.roles\n", 1);
    let s = SchoolConfig::from_yaml(&yaml).unwrap();
    assert_eq!(s.auth.roles_claim, "entitlements.roles");
    s.validate(&env(false)).unwrap();
    for bad in ["", "entitlements.", ".roles", "a..b"] {
        let mut s = school();
        s.auth.roles_claim = bad.into();
        assert_eq!(
            school_issues(&s, &env(false)),
            ["auth.roles_claim"],
            "{bad:?}"
        );
    }
}

#[test]
fn unknown_role_and_class_are_parse_errors() {
    let yaml = TEMPLE_POLICY.replacen("roles: [student]", "roles: [provost]", 1);
    assert!(matches!(
        PolicyConfig::from_yaml(&yaml),
        Err(ConfigError::Parse { .. })
    ));
    let yaml = TEMPLE_POLICY.replacen(
        "effective_classes: [public]",
        "effective_classes: [secret]",
        1,
    );
    assert!(matches!(
        PolicyConfig::from_yaml(&yaml),
        Err(ConfigError::Parse { .. })
    ));
}

#[test]
fn missing_secret_reference_fails() {
    let no_vars = Env {
        has_var: &|_| false,
        sandbox_compiled: false,
    };
    assert_eq!(policy_issues(&policy(), &no_vars), ["audit.salt_env"]);

    let mut p = policy();
    p.audit.salt_env = "not an env name".into();
    assert_eq!(policy_issues(&p, &env(false)), ["audit.salt_env"]);
}

#[test]
fn diagnostics_never_echo_secret_values() {
    let no_vars = Env {
        has_var: &|_| false,
        sandbox_compiled: false,
    };
    let err = policy()
        .validate(&school(), &tools(), &no_vars)
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("KAMPUS_TEMPLE_AUDIT_SALT"));
    assert!(text.contains("not set"));
}

#[test]
fn only_pre_registered_confidential_clients_can_be_approved() {
    for kind in [
        ClientRegistration::Dcr,
        ClientRegistration::Cimd,
        ClientRegistration::Device,
    ] {
        let mut p = policy();
        p.client_registry[1].registration = kind;
        p.client_registry[1].approved = true;
        assert_eq!(
            policy_issues(&p, &env(false)),
            ["client_registry[1].approved"]
        );
    }
}

#[test]
fn invalid_tiers_fail() {
    let mut p = policy();
    p.tier_mapping
        .insert(DataClass::Sensitive, DataClass::Public);
    assert_eq!(policy_issues(&p, &env(false)), ["tier_mapping.sensitive"]);

    let mut p = policy();
    p.tier_mapping.remove(&DataClass::Confidential);
    assert_eq!(
        policy_issues(&p, &env(false)),
        ["tier_mapping.confidential"]
    );
}

#[test]
fn cross_school_policy_fails() {
    let mut p = policy();
    p.school_id = "sju".into();
    assert_eq!(policy_issues(&p, &env(false)), ["school_id"]);
}

#[test]
fn unknown_tools_and_duplicate_rules_fail() {
    let mut p = policy();
    p.rules[0].tools.insert("raw_libcal_proxy".into());
    assert_eq!(policy_issues(&p, &env(false)), ["rules[0].tools"]);

    let mut p = policy();
    p.rules[1].id = p.rules[0].id.clone();
    assert_eq!(policy_issues(&p, &env(false)), ["rules"]);
}

#[test]
fn confidential_can_never_be_allowed() {
    let mut p = policy();
    p.rules[0].effective_classes.insert(DataClass::Confidential);
    assert_eq!(
        policy_issues(&p, &env(false)),
        ["rules[0].effective_classes"]
    );
}

#[test]
fn unsafe_urls_fail() {
    let mut s = school();
    s.libcal.as_mut().unwrap().base_url = "http://charlesstudy.temple.edu".parse().unwrap();
    assert_eq!(school_issues(&s, &env(false)), ["libcal.base_url"]);

    let mut s = school();
    s.banner.as_mut().unwrap().base_url = "https://user:pw@prd-xereg.temple.edu/".parse().unwrap();
    assert_eq!(school_issues(&s, &env(false)), ["banner.base_url"]);

    let mut s = school();
    s.server.resource_url = "https://kampus.change-me.invalid/mcp".parse().unwrap();
    assert_eq!(school_issues(&s, &env(false)), ["server.resource_url"]);
}

#[test]
fn admin_listener_must_be_loopback() {
    let mut s = school();
    s.server.admin_listen = "0.0.0.0:8081".parse().unwrap();
    assert_eq!(school_issues(&s, &env(false)), ["server.admin_listen"]);
}

#[test]
fn production_rejects_mocks_and_replay() {
    let mut s = school();
    s.libcal.as_mut().unwrap().booking_simulation = true;
    s.transit.as_mut().unwrap().replay_path = Some("fixtures/flight/replay".into());
    let issues = school_issues(&s, &env(false));
    assert_eq!(issues, ["libcal.booking_simulation", "transit.replay_path"]);

    // Runtime opt-in alone is not enough without the compiled feature.
    s.sandbox.enabled = true;
    let issues = school_issues(&s, &env(false));
    assert!(issues.contains(&"sandbox.enabled".to_owned()));

    // A sandbox build with runtime opt-in accepts them.
    s.validate(&env(true)).unwrap();
}

#[test]
fn sandbox_build_still_needs_runtime_opt_in() {
    let mut s = school();
    s.libcal.as_mut().unwrap().booking_simulation = true;
    assert_eq!(school_issues(&s, &env(true)), ["libcal.booking_simulation"]);
}

#[test]
fn device_stickers_must_be_synthetic_and_devices_unapproved() {
    let mut s = school();
    s.devices.push(DeviceConfig {
        device_id: "owl-1".into(),
        client_id: "kampus-mascot".into(),
        bind_secret_env: "KAMPUS_OWL_BIND_SECRET".into(),
        stickers: vec![DeviceSticker {
            uid: "04:A2:19".into(),
            subject: "tuj12345".into(),
            role: Role::Admin,
        }],
    });
    assert_eq!(
        school_issues(&s, &env(false)),
        [
            "devices[0].stickers[0].subject",
            "devices[0].stickers[0].role"
        ]
    );

    s.devices[0].stickers[0].subject = "synthetic:student-a".into();
    s.devices[0].stickers[0].role = Role::Student;
    s.validate(&env(false)).unwrap();

    // The device client_id must be registered as a device client, never approved.
    let mut p = policy();
    p.client_registry.retain(|c| c.client_id != "kampus-mascot");
    match p.validate(&s, &tools(), &env(false)) {
        Err(ConfigError::Invalid(issues)) => assert_eq!(issues[0].field, "devices[0].client_id"),
        other => panic!("expected device client_id issue, got {other:?}"),
    }
}

#[test]
fn defaults_match_frozen_values() {
    let limits = PolicyLimits::default();
    assert_eq!(limits.user_reads_per_minute, 60);
    assert_eq!(limits.guest_reads_per_minute, 30);
    let upstream = UpstreamLimits::default();
    assert_eq!(
        (
            upstream.requests_per_second,
            upstream.burst,
            upstream.timeout_ms
        ),
        (1, 2, 5000)
    );
}

// ---- trusted proxy and admission limits -------------------------------------

fn proxy(header: ForwardedHeader, peers: &[&str]) -> TrustedProxy {
    TrustedProxy {
        header,
        peers: peers.iter().map(|p| (*p).to_owned()).collect(),
    }
}

fn ip(text: &str) -> std::net::IpAddr {
    text.parse().unwrap()
}

#[test]
fn forwarded_headers_count_only_from_trusted_peers() {
    let cf = proxy(
        ForwardedHeader::CfConnectingIp,
        &["127.0.0.1", "10.0.0.0/8"],
    );
    // Untrusted peer: the header is ignored.
    assert_eq!(
        cf.client_ip(ip("198.51.100.9"), ["203.0.113.5"]),
        ip("198.51.100.9")
    );
    // Trusted peers (exact and CIDR): the header is used.
    assert_eq!(
        cf.client_ip(ip("127.0.0.1"), ["203.0.113.5"]),
        ip("203.0.113.5")
    );
    assert_eq!(
        cf.client_ip(ip("10.20.30.40"), ["2001:db8::7"]),
        ip("2001:db8::7")
    );
    // Missing, garbage or duplicated values fall back to the peer.
    assert_eq!(cf.client_ip(ip("127.0.0.1"), []), ip("127.0.0.1"));
    assert_eq!(
        cf.client_ip(ip("127.0.0.1"), ["not-an-ip"]),
        ip("127.0.0.1")
    );
    assert_eq!(
        cf.client_ip(ip("127.0.0.1"), ["203.0.113.5", "203.0.113.6"]),
        ip("127.0.0.1")
    );
    // An IPv4-mapped peer is the same peer.
    assert_eq!(
        cf.client_ip(ip("::ffff:127.0.0.1"), ["203.0.113.5"]),
        ip("203.0.113.5")
    );
}

#[test]
fn x_forwarded_for_uses_the_rightmost_untrusted_hop() {
    let xff = proxy(ForwardedHeader::XForwardedFor, &["127.0.0.1", "10.0.0.0/8"]);
    // A client-supplied spoofed hop on the left is skipped.
    assert_eq!(
        xff.client_ip(ip("127.0.0.1"), ["6.6.6.6, 203.0.113.5, 10.1.1.1"]),
        ip("203.0.113.5")
    );
    // Multiple header lines form one list.
    assert_eq!(
        xff.client_ip(ip("127.0.0.1"), ["6.6.6.6", "203.0.113.5"]),
        ip("203.0.113.5")
    );
    assert_eq!(
        xff.client_ip(ip("127.0.0.1"), ["garbage, 1.2.3.4"]),
        ip("127.0.0.1")
    );
    assert_eq!(
        xff.client_ip(ip("192.0.2.1"), ["203.0.113.5"]),
        ip("192.0.2.1")
    );
}

#[test]
fn trusted_proxy_is_optional_and_validated() {
    assert_eq!(school().server.trusted_proxy, None);
    let mut s = school();
    s.server.trusted_proxy = Some(proxy(
        ForwardedHeader::CfConnectingIp,
        &["127.0.0.1", "::1", "10.0.0.0/8", "fd00::/8"],
    ));
    s.validate(&env(false)).unwrap();
    s.server.trusted_proxy = Some(proxy(ForwardedHeader::XForwardedFor, &[]));
    assert_eq!(
        school_issues(&s, &env(false)),
        ["server.trusted_proxy.peers"]
    );
    s.server.trusted_proxy = Some(proxy(
        ForwardedHeader::XForwardedFor,
        &["127.0.0.1", "10.0.0.0/33", "example.org"],
    ));
    assert_eq!(
        school_issues(&s, &env(false)),
        [
            "server.trusted_proxy.peers[1]",
            "server.trusted_proxy.peers[2]"
        ]
    );
    let yaml = TEMPLE_SCHOOL.replacen(
        "server:\n",
        "server:\n  trusted_proxy: { header: x_forwarded_for, peers: [\"127.0.0.1\"] }\n",
        1,
    );
    let parsed = SchoolConfig::from_yaml(&yaml).unwrap();
    assert_eq!(
        parsed.server.trusted_proxy.map(|p| p.header),
        Some(ForwardedHeader::XForwardedFor)
    );
}

#[test]
fn admission_limits_default_and_must_be_positive() {
    let p = policy();
    assert_eq!(p.limits.calls_per_minute, 120);
    assert_eq!(p.limits.max_concurrent_calls, 64);
    let mut zero = policy();
    zero.limits.max_concurrent_calls = 0;
    assert_eq!(policy_issues(&zero, &env(false)), ["limits"]);
}
