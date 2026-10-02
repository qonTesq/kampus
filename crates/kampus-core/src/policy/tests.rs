//! Policy matrix against the real Temple `policy.yaml` and the real module
//! descriptors (`modules/*/module.yaml`).

use std::collections::BTreeSet;

use super::*;
use crate::config::{ClientRegistration, RegisteredClient};
use crate::contracts::{ModuleDescriptor, Role};

const TEMPLE_POLICY: &str = include_str!("../../../../schools/temple/policy.yaml");

fn temple() -> PolicyConfig {
    PolicyConfig::from_yaml(TEMPLE_POLICY).unwrap()
}

fn demo() -> PolicyConfig {
    let mut p = temple();
    p.sensitive_enabled = true;
    p
}

fn tool(name: &str) -> ToolDescriptor {
    let modules = [
        (
            "rooms",
            include_str!("../../../../modules/rooms/module.yaml"),
        ),
        (
            "courses",
            include_str!("../../../../modules/courses/module.yaml"),
        ),
        (
            "transit",
            include_str!("../../../../modules/transit/module.yaml"),
        ),
        (
            "library_hours",
            include_str!("../../../../modules/library-hours/module.yaml"),
        ),
        (
            "deadlines",
            include_str!("../../../../modules/deadlines/module.yaml"),
        ),
    ];
    modules
        .iter()
        .flat_map(|(id, yaml)| ModuleDescriptor::from_yaml(id, yaml).unwrap().tools)
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no tool {name}"))
}

fn operation(name: &str) -> Operation {
    match name {
        "confirm_booking" => Operation::Confirm,
        "book_study_room" => Operation::Propose,
        _ => Operation::Read,
    }
}

fn user(client_id: &str, roles: &[Role], scopes: &[&str]) -> Principal {
    Principal {
        subject: "synthetic-subject".into(),
        client_id: Some(client_id.into()),
        roles: roles.iter().copied().collect(),
        scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
        auth_kind: AuthKind::User,
        device_id: None,
    }
}

fn student(client_id: &str) -> Principal {
    user(client_id, &[Role::Student], &[])
}

fn device(roles: &[Role]) -> Principal {
    Principal {
        auth_kind: AuthKind::Device,
        device_id: Some("owl-1".into()),
        ..user("kampus-mascot", roles, &["rooms:book"])
    }
}

fn decide(policy: &PolicyConfig, who: &Principal, name: &str) -> Decision {
    decide_bound(policy, who, name, None)
}

fn decide_bound(
    policy: &PolicyConfig,
    who: &Principal,
    name: &str,
    binding: Option<u64>,
) -> Decision {
    let revision = PolicyRevision {
        config: policy.clone(),
        version: 7,
    };
    PolicyEngine::evaluate_with(&revision, who, binding, &tool(name), operation(name))
}

fn allowed(policy: &PolicyConfig, who: &Principal, name: &str) -> bool {
    decide(policy, who, name).allowed
}

const PUBLIC_READS: [&str; 4] = [
    "find_study_room",
    "search_courses",
    "next_departures",
    "library_hours",
];

#[test]
fn guest_reads_public_and_nothing_else() {
    let guest = Principal::guest();
    for name in PUBLIC_READS {
        assert!(allowed(&demo(), &guest, name), "{name}");
    }
    assert_eq!(
        decide(&demo(), &guest, "book_study_room").rule_id,
        rules::TOOL_ROLES
    );
    assert_eq!(
        decide(&demo(), &guest, "confirm_booking").rule_id,
        rules::TOOL_ROLES
    );
    assert_eq!(
        decide(&demo(), &guest, "my_deadlines").rule_id,
        rules::TOOL_ROLES
    );
}

#[test]
fn unapproved_student_gets_public_and_proposal_but_not_sensitive() {
    for client_id in ["kampus-chatgpt", "dcr-4f2a", "some-unknown-client"] {
        let s = student(client_id);
        for name in PUBLIC_READS {
            assert!(allowed(&demo(), &s, name), "{client_id} {name}");
        }
        // book_study_room: student-only proposal, no write scope needed.
        assert_eq!(
            decide(&demo(), &s, "book_study_room").rule_id,
            "booking-propose"
        );
        let d = decide(&demo(), &s, "my_deadlines");
        assert!(!d.allowed);
        assert_eq!(d.rule_id, rules::SENSITIVE_CLIENT, "{client_id}");
        assert_eq!(d.effective_class, DataClass::Sensitive);
        assert_eq!(d.policy_version, 7);
    }
}

#[test]
fn approved_student_gets_sensitive_only_when_enabled() {
    let s = student("kampus-claude");
    assert_eq!(
        decide(&temple(), &s, "my_deadlines").rule_id,
        rules::SENSITIVE_DISABLED
    );
    let d = decide(&demo(), &s, "my_deadlines");
    assert!(d.allowed);
    assert_eq!(d.rule_id, "sensitive-approved-student");
}

#[test]
fn confirm_needs_student_and_rooms_book() {
    let without = student("kampus-claude");
    assert_eq!(
        decide(&demo(), &without, "confirm_booking").rule_id,
        rules::TOOL_SCOPES
    );
    let with = user("kampus-chatgpt", &[Role::Student], &["rooms:book"]);
    assert_eq!(
        decide(&demo(), &with, "confirm_booking").rule_id,
        "booking-confirm"
    );
    let faculty = user("kampus-claude", &[Role::Faculty], &["rooms:book"]);
    assert_eq!(
        decide(&demo(), &faculty, "confirm_booking").rule_id,
        rules::TOOL_ROLES
    );
}

#[test]
fn faculty_staff_and_admin() {
    for role in [Role::Faculty, Role::Staff] {
        let p = user("kampus-claude", &[role], &[]);
        assert!(allowed(&demo(), &p, "next_departures"));
        assert!(!allowed(&demo(), &p, "book_study_room"));
        assert!(!allowed(&demo(), &p, "my_deadlines"));
    }
    // Admin is local administration only, never implicit data access.
    let admin = user("kampus-claude", &[Role::Admin], &["rooms:book"]);
    for name in PUBLIC_READS
        .iter()
        .chain(&["book_study_room", "confirm_booking", "my_deadlines"])
    {
        assert!(!allowed(&demo(), &admin, name), "{name}");
    }
}

#[test]
fn self_registered_clients_cannot_borrow_approval() {
    // A DCR/CIMD client copying the approved workspace name, even marked approved.
    let mut policy = demo();
    for (client_id, registration) in [
        ("dcr-copy", ClientRegistration::Dcr),
        ("https://cimd.example/client", ClientRegistration::Cimd),
    ] {
        policy.client_registry.push(RegisteredClient {
            client_id: client_id.into(),
            registration,
            approved: true,
            workspace: "Claude (school workspace)".into(),
        });
        let d = decide(&policy, &student(client_id), "my_deadlines");
        assert_eq!(
            d.rule_id,
            rules::SELF_REGISTERED_SENSITIVE_OFF,
            "{client_id}"
        );
        assert_eq!(
            client_class(&policy, &student(client_id)),
            ClientClass::SelfRegistered
        );
    }
    // A pre-registered client_id presented through a device token is still a device.
    let mut spoof = device(&[Role::Student]);
    spoof.client_id = Some("kampus-claude".into());
    assert!(!allowed(&policy, &spoof, "my_deadlines"));
}

#[test]
fn devices_never_get_sensitive_and_need_a_live_binding_to_book() {
    let unbound = device(&[Role::Guest]);
    assert!(allowed(&demo(), &unbound, "next_departures"));
    assert!(!allowed(&demo(), &unbound, "book_study_room"));
    assert!(!allowed(&demo(), &unbound, "confirm_booking"));

    let bound = device(&[Role::Student]);
    assert_eq!(
        decide_bound(&demo(), &bound, "book_study_room", None).rule_id,
        rules::DEVICE_UNBOUND
    );
    assert!(decide_bound(&demo(), &bound, "book_study_room", Some(3)).allowed);
    assert!(decide_bound(&demo(), &bound, "confirm_booking", Some(3)).allowed);
    assert_eq!(
        decide_bound(&demo(), &bound, "my_deadlines", Some(3)).rule_id,
        rules::SENSITIVE_CLIENT
    );
}

#[test]
fn deny_wins_and_unmatched_denies() {
    let mut policy = demo();
    policy.rules.insert(
        1,
        Rule {
            id: "no-transit-tonight".into(),
            effect: Effect::Deny,
            roles: BTreeSet::from([Role::Student]),
            tools: BTreeSet::from(["next_departures".to_owned()]),
            effective_classes: BTreeSet::from([DataClass::Public]),
            client_trust: ClientTrust::Any,
            required_scopes: BTreeSet::new(),
            enabled: true,
        },
    );
    assert_eq!(
        decide(&policy, &student("kampus-claude"), "next_departures").rule_id,
        "no-transit-tonight"
    );
    assert!(allowed(&policy, &Principal::guest(), "next_departures"));

    let mut empty = demo();
    empty.rules.clear();
    assert_eq!(
        decide(&empty, &student("kampus-claude"), "next_departures").rule_id,
        rules::DEFAULT_DENY
    );
}

#[test]
fn disabled_rules_do_not_match() {
    let mut policy = demo();
    for rule in &mut policy.rules {
        if rule.id == "sensitive-approved-student" {
            rule.enabled = false;
        }
    }
    assert_eq!(
        decide(&policy, &student("kampus-claude"), "my_deadlines").rule_id,
        rules::DEFAULT_DENY
    );
}

#[test]
fn internal_maps_to_sensitive_and_confidential_is_never_callable() {
    let mut internal = tool("my_deadlines");
    internal.data_class = DataClass::Internal;
    let revision = PolicyRevision {
        config: demo(),
        version: 1,
    };
    let d = PolicyEngine::evaluate_with(
        &revision,
        &student("kampus-chatgpt"),
        None,
        &internal,
        Operation::Read,
    );
    assert_eq!(d.effective_class, DataClass::Sensitive);
    assert_eq!(d.rule_id, rules::SENSITIVE_CLIENT);

    let mut secret = tool("next_departures");
    secret.data_class = DataClass::Confidential;
    let d = PolicyEngine::evaluate_with(
        &revision,
        &student("kampus-claude"),
        None,
        &secret,
        Operation::Read,
    );
    assert_eq!(d.rule_id, rules::CONFIDENTIAL);
}

#[test]
fn swaps_are_atomic_and_versioned() {
    let engine = PolicyEngine::new(demo());
    let s = student("kampus-chatgpt");
    let ctx = CallContext {
        school_id: "temple".into(),
        principal: s,
        request_id: "r1".into(),
        now: chrono::Utc::now(),
        policy_version: 0,
        binding_generation: None,
    };
    let deadlines = tool("my_deadlines");
    let before = engine.evaluate(&ctx, &deadlines, Operation::Read);
    assert!(!before.allowed);
    assert_eq!(before.policy_version, 1);

    // The demo switch: approve the existing pre-registered ChatGPT entry.
    let mut next = demo();
    for c in &mut next.client_registry {
        if c.client_id == "kampus-chatgpt" {
            c.approved = true;
        }
    }
    assert_eq!(engine.replace(next), 2);
    let after = engine.evaluate(&ctx, &deadlines, Operation::Read);
    assert!(after.allowed);
    assert_eq!(after.policy_version, 2);
}

// ---- client classes and the personal switch --------------------------------

const CIMD: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";

fn personal() -> PolicyConfig {
    let mut p = demo();
    p.allow_self_registered_sensitive = true;
    p
}

#[test]
fn clients_are_classified_from_the_registry_only() {
    let p = temple();
    assert_eq!(
        client_class(&p, &student("kampus-claude")),
        ClientClass::Approved
    );
    assert_eq!(
        client_class(&p, &student("kampus-chatgpt")),
        ClientClass::Other
    );
    assert_eq!(
        client_class(&p, &student(CIMD)),
        ClientClass::SelfRegistered
    );
    // An unregistered HTTPS client ID is just another unknown client.
    assert_eq!(
        client_class(&p, &student("https://assistant.example/client.json")),
        ClientClass::Other
    );
    assert_eq!(client_class(&p, &Principal::guest()), ClientClass::Other);
    let mut cimd_device = device(&[Role::Student]);
    cimd_device.client_id = Some(CIMD.into());
    assert_eq!(client_class(&p, &cimd_device), ClientClass::Other);
}

#[test]
fn personal_switch_off_has_its_own_rule() {
    let d = decide(&demo(), &student(CIMD), "my_deadlines");
    assert!(!d.allowed);
    assert_eq!(d.rule_id, rules::SELF_REGISTERED_SENSITIVE_OFF);
    // The global Sensitive switch still comes first.
    let mut p = temple();
    p.allow_self_registered_sensitive = true;
    assert_eq!(
        decide(&p, &student(CIMD), "my_deadlines").rule_id,
        rules::SENSITIVE_DISABLED
    );
}

#[test]
fn personal_switch_on_allows_a_student_read_through_the_explicit_rule() {
    let d = decide(&personal(), &student(CIMD), "my_deadlines");
    assert!(d.allowed);
    assert_eq!(d.rule_id, "sensitive-self-registered-student");
    // Approved and other clients are unaffected by the switch.
    assert_eq!(
        decide(&personal(), &student("kampus-claude"), "my_deadlines").rule_id,
        "sensitive-approved-student"
    );
    for other in ["kampus-chatgpt", "https://assistant.example/client.json"] {
        assert_eq!(
            decide(&personal(), &student(other), "my_deadlines").rule_id,
            rules::SENSITIVE_CLIENT,
            "{other}"
        );
    }
}

/// `my_deadlines` reshaped to reach the switch's own limits past the
/// descriptor's role and operation checks.
fn decide_reshaped(
    policy: &PolicyConfig,
    who: &Principal,
    roles: &[Role],
    operation: Operation,
) -> Decision {
    let mut t = tool("my_deadlines");
    t.roles = roles.iter().copied().collect();
    let revision = PolicyRevision {
        config: policy.clone(),
        version: 7,
    };
    PolicyEngine::evaluate_with(&revision, who, Some(1), &t, operation)
}

#[test]
fn limits_still_hold_under_the_switch() {
    let p = personal();
    let all = [Role::Student, Role::Faculty, Role::Guest];
    // A faculty member, a device and a write.
    let faculty = user(CIMD, &[Role::Faculty], &[]);
    assert_eq!(
        decide_reshaped(&p, &faculty, &all, Operation::Read).rule_id,
        rules::SENSITIVE_CLIENT
    );
    let mut cimd_device = device(&[Role::Student]);
    cimd_device.client_id = Some(CIMD.into());
    assert_eq!(
        decide_reshaped(&p, &cimd_device, &all, Operation::Read).rule_id,
        rules::SENSITIVE_CLIENT
    );
    for write in [Operation::Propose, Operation::Confirm] {
        assert_eq!(
            decide_reshaped(&p, &student(CIMD), &all, write).rule_id,
            rules::SENSITIVE_CLIENT
        );
    }
    // A `*` self-registered rule, or a broader rule, never allows.
    let mut wildcard = personal();
    for rule in &mut wildcard.rules {
        if rule.id == "sensitive-self-registered-student" {
            rule.tools = BTreeSet::from(["*".to_owned()]);
        }
    }
    wildcard.rules.push(Rule {
        id: "everyone-sensitive".into(),
        effect: Effect::Allow,
        roles: BTreeSet::from([Role::Student]),
        tools: BTreeSet::from(["my_deadlines".to_owned()]),
        effective_classes: BTreeSet::from([DataClass::Sensitive]),
        client_trust: ClientTrust::Any,
        required_scopes: BTreeSet::new(),
        enabled: true,
    });
    assert_eq!(
        decide(&wildcard, &student(CIMD), "my_deadlines").rule_id,
        rules::DEFAULT_DENY
    );
    // Confidential stays uncallable.
    let mut confidential = personal();
    confidential
        .tier_mapping
        .insert(DataClass::Sensitive, DataClass::Confidential);
    assert_eq!(
        decide(&confidential, &student(CIMD), "my_deadlines").rule_id,
        rules::CONFIDENTIAL
    );
    // An explicit deny wins.
    let mut denied = personal();
    denied.rules.insert(
        0,
        Rule {
            id: "no-personal-deadlines".into(),
            effect: Effect::Deny,
            roles: BTreeSet::from([Role::Student]),
            tools: BTreeSet::from(["my_deadlines".to_owned()]),
            effective_classes: BTreeSet::from([DataClass::Sensitive]),
            client_trust: ClientTrust::SelfRegistered,
            required_scopes: BTreeSet::new(),
            enabled: true,
        },
    );
    assert_eq!(
        decide(&denied, &student(CIMD), "my_deadlines").rule_id,
        "no-personal-deadlines"
    );
}

#[test]
fn trust_selectors_split_unapproved_callers() {
    let mut p = personal();
    p.rules.clear();
    for (id, trust) in [
        ("only-self", ClientTrust::SelfRegistered),
        ("only-other", ClientTrust::UnapprovedOther),
    ] {
        p.rules.push(Rule {
            id: id.into(),
            effect: Effect::Allow,
            roles: BTreeSet::from([Role::Student, Role::Guest]),
            tools: BTreeSet::from(["next_departures".to_owned()]),
            effective_classes: BTreeSet::from([DataClass::Public]),
            client_trust: trust,
            required_scopes: BTreeSet::new(),
            enabled: true,
        });
    }
    let rule = |who: &Principal| decide(&p, who, "next_departures").rule_id;
    assert_eq!(rule(&student(CIMD)), "only-self");
    assert_eq!(rule(&student("kampus-chatgpt")), "only-other");
    assert_eq!(rule(&Principal::guest()), "only-other");
    assert_eq!(rule(&student("kampus-claude")), rules::DEFAULT_DENY);
}
