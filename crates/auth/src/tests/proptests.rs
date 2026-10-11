//! Task 5.5: the design §14 proptest invariants.
//!
//! (a) no sequence of refresh, revoke and reuse yields a working token after
//!     its family or grant is revoked; (b) a refresh never returns a scope
//!     outside the grant; (c) consent for S never satisfies S' != S;
//! (d) any single-character mutation of a registered redirect URI is
//!     rejected; (e) scope parsing is idempotent and order-insensitive.

use std::collections::BTreeSet;

use ::proptest::prelude::*;
use ::proptest::test_runner::{Config, TestRunner};

use super::{AuthRequest, CLAUDE_REDIRECT, Harness};
use crate::authorize::{AuthorizeQuery, Refusal};
use crate::pages::PageError;
use crate::store::{Capacity, Family, GrantKey, MemoryStore, Rotation};

const UNIVERSE: [&str; 4] = ["a", "b", "rooms:book", "c:d"];

fn set(mask: u8) -> BTreeSet<String> {
    UNIVERSE
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(_, s)| (*s).to_owned())
        .collect()
}

fn key(subject: u8, mask: u8) -> GrantKey {
    GrantKey {
        subject: format!("synthetic:s{}", subject % 2),
        client_id: "https://claude.ai/oauth/mcp-oauth-client-metadata".into(),
        resource: super::RESOURCE.into(),
        scopes: set(mask),
    }
}

#[derive(Debug, Clone)]
enum Op {
    Grant(u8, u8),
    Family(usize),
    Refresh(usize),
    Reuse(usize, usize),
    Revoke(usize),
    RevokeAsOther(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u8>(), any::<u8>()).prop_map(|(s, m)| Op::Grant(s, m)),
        any::<usize>().prop_map(Op::Family),
        any::<usize>().prop_map(Op::Refresh),
        (any::<usize>(), any::<usize>()).prop_map(|(f, t)| Op::Reuse(f, t)),
        any::<usize>().prop_map(Op::Revoke),
        any::<usize>().prop_map(Op::RevokeAsOther),
    ]
}

struct ModelGrant {
    id: String,
    key: GrantKey,
    alive: bool,
}

struct ModelFamily {
    key: GrantKey,
    current: String,
    retired: Vec<String>,
    dead: bool,
}

const NOW: i64 = 100;

proptest! {
    /// (a) Revoked families and grants never come back.
    #[test]
    fn revoked_families_and_grants_stay_dead(ops in prop::collection::vec(op(), 1..60)) {
        let store = MemoryStore::new(Capacity::DEFAULT);
        let mut grants: Vec<ModelGrant> = Vec::new();
        let mut families: Vec<ModelFamily> = Vec::new();
        let mut serial = 0u32;
        let mut token = || {
            serial += 1;
            format!("t{serial}")
        };
        for op in ops {
            match op {
                Op::Grant(s, m) => {
                    let grant = store.upsert_grant(key(s, m), "l".into(), NOW).unwrap();
                    if !grants.iter().any(|g| g.id == grant.id) {
                        grants.push(ModelGrant { id: grant.id, key: grant.key, alive: true });
                    }
                }
                Op::Family(i) if !grants.is_empty() => {
                    let g = &grants[i % grants.len()];
                    if !g.alive {
                        continue;
                    }
                    let current = token();
                    let id = format!("f{}", families.len());
                    store
                        .insert_family(Family {
                            id,
                            grant: g.key.clone(),
                            roles: BTreeSet::new(),
                            current: current.clone(),
                            expires_at: NOW + 1000,
                        })
                        .unwrap();
                    families.push(ModelFamily { key: g.key.clone(), current, retired: Vec::new(), dead: false });
                }
                Op::Refresh(i) if !families.is_empty() => {
                    let n = families.len();
                    let f = &mut families[i % n];
                    let next = token();
                    let outcome = store.rotate(&f.current, next.clone(), NOW);
                    if f.dead {
                        prop_assert!(!matches!(outcome, Rotation::Rotated(_)));
                    } else {
                        prop_assert!(matches!(outcome, Rotation::Rotated(_)), "{outcome:?}");
                        f.retired.push(std::mem::replace(&mut f.current, next));
                    }
                }
                Op::Reuse(i, j) if !families.is_empty() => {
                    let n = families.len();
                    let f = &mut families[i % n];
                    if f.retired.is_empty() {
                        continue;
                    }
                    let presented = f.retired[j % f.retired.len()].clone();
                    let outcome = store.rotate(&presented, token(), NOW);
                    prop_assert!(!matches!(outcome, Rotation::Rotated(_)));
                    f.dead = true;
                }
                Op::Revoke(i) if !grants.is_empty() => {
                    let n = grants.len();
                    let g = &mut grants[i % n];
                    let revoked = store.revoke_grant(&g.key.subject, &g.id);
                    prop_assert_eq!(revoked.is_some(), g.alive);
                    // Revoking an already revoked ID touches nothing, not even
                    // a newer grant for the same key.
                    if g.alive {
                        g.alive = false;
                        for f in families.iter_mut().filter(|f| f.key == g.key) {
                            f.dead = true;
                        }
                    }
                }
                Op::RevokeAsOther(i) if !grants.is_empty() => {
                    let g = &grants[i % grants.len()];
                    prop_assert!(store.revoke_grant("synthetic:intruder", &g.id).is_none());
                }
                _ => {}
            }
            for f in &families {
                let live = store.family_by_token(&f.current, NOW).is_some();
                prop_assert_eq!(live, !f.dead);
                for t in &f.retired {
                    prop_assert!(store.family_by_token(t, NOW).is_none());
                }
            }
        }
    }

    /// (b) Whatever a refresh asks for, it gets a subset of the grant.
    #[test]
    fn a_refresh_never_widens_the_grant(
        grant in 0u8..16,
        asked in prop::collection::vec(prop::sample::select(vec![
            "a", "b", "rooms:book", "c:d", "offline_access", "admin", "x\"y",
        ]), 0..6),
        send in any::<bool>(),
    ) {
        let grant = set(grant);
        let requested = asked.join(" ");
        let outcome = crate::token::refresh_scopes(send.then_some(requested.as_str()), &grant);
        if let Ok(scopes) = outcome {
            prop_assert!(scopes.is_subset(&grant), "{scopes:?} ⊄ {grant:?}");
            prop_assert!(!scopes.contains("offline_access"));
        }
        let wider = asked
            .iter()
            .any(|s| *s != "offline_access" && !grant.contains(*s));
        if send && wider {
            prop_assert!(crate::token::refresh_scopes(Some(&requested), &grant).is_err());
        }
    }

    /// (c) A grant for S is found only for exactly S.
    #[test]
    fn consent_for_one_scope_set_never_covers_another(
        subject in any::<u8>(),
        granted in 0u8..16,
        asked in 0u8..16,
    ) {
        let store = MemoryStore::new(Capacity::DEFAULT);
        store.upsert_grant(key(subject, granted), "l".into(), NOW).unwrap();
        let found = store.grant(&key(subject, asked)).is_some();
        prop_assert_eq!(found, granted == asked);
        // Another subject never shares it.
        prop_assert!(store.grant(&key(subject.wrapping_add(1), granted)).is_none());
    }

    /// (e) Parsing ignores order and duplicates, and re-parsing is a no-op.
    #[test]
    fn scope_parsing_is_idempotent_and_order_insensitive(
        tokens in prop::collection::vec("[a-z:_.-]{1,12}", 0..8)
            .prop_flat_map(|t| (Just(t.clone()), Just(t).prop_shuffle())),
        gaps in prop::collection::vec(1usize..4, 8),
        noise in "[ -~]{0,40}",
    ) {
        let (tokens, shuffled) = tokens;
        let join = |tokens: &[String]| {
            tokens
                .iter()
                .zip(&gaps)
                .flat_map(|(t, g)| [t.clone(), " ".repeat(*g)])
                .collect::<String>()
        };
        let parsed = crate::scopes::parse(&join(&tokens)).unwrap();
        let expected: BTreeSet<String> = tokens.iter().cloned().collect();
        prop_assert_eq!(&parsed, &expected);
        prop_assert_eq!(crate::scopes::parse(&join(&shuffled)).unwrap(), parsed);
        // Arbitrary input: when it parses, its canonical form parses back.
        if let Some(set) = crate::scopes::parse(&noise) {
            let canonical = set.iter().cloned().collect::<Vec<_>>().join(" ");
            prop_assert_eq!(crate::scopes::parse(&canonical), Some(set));
        }
    }
}

/// A single-character edit of `uri`: replace, insert or delete at `at`.
fn mutate(uri: &str, at: usize, kind: u8, with: char) -> String {
    let mut chars: Vec<char> = uri.chars().collect();
    let at = at % (chars.len() + 1);
    match kind % 3 {
        0 if at < chars.len() => chars[at] = with,
        1 => chars.insert(at, with),
        _ if at < chars.len() => {
            chars.remove(at);
        }
        _ => chars.push(with),
    }
    chars.into_iter().collect()
}

/// (d) Through the real authorization validation: every single-character
/// mutation of Claude's registered redirect is an error page, never a
/// redirect to the mutated address.
#[test]
fn any_one_character_redirect_mutation_is_rejected() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let harness = runtime.block_on(Harness::start());
    let inner = &harness.broker.inner;
    // The unmutated request is accepted, so refusals below are about the URI.
    let ok = AuthorizeQuery::parse(&AuthRequest::claude().query()).unwrap();
    assert!(runtime.block_on(inner.validate(&ok)).is_ok());
    let mut runner = TestRunner::new(Config::default());
    let strategy = (any::<usize>(), any::<u8>(), any::<char>());
    runner
        .run(&strategy, |(at, kind, with)| {
            let mutated = mutate(CLAUDE_REDIRECT, at, kind, with);
            prop_assume!(mutated != CLAUDE_REDIRECT);
            let request = AuthRequest::claude().set("redirect_uri", Some(&mutated));
            let query = AuthorizeQuery::parse(&request.query()).unwrap();
            let outcome = runtime.block_on(inner.validate(&query));
            prop_assert_eq!(outcome, Err(Refusal::Page(PageError::InvalidRedirect)));
            Ok(())
        })
        .unwrap();
}
