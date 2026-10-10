//! Sandbox two-step booking: in-memory, identity-bound, single-use proposals.
//!
//! Compiled only with the `sandbox` feature and active only when the school
//! sets `sandbox.enabled` and `libcal.booking_simulation`. Proposals live in
//! memory only, so a restart discards them. Nothing here is logged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use kampus_core::contracts::providers::{BookingProposal, BookingProvider, TimeSlot};
use kampus_core::contracts::{
    CallContext, ErrorCode, Provenance, Role, SourceKind, ToolError, ToolSuccess, parse_args,
};
use kampus_core::runtime::RateLimiter;
use kampus_libcal::{Freshness, LibCal, SimulatedBooking, random_id};
use serde::Serialize;
use serde_json::Value;

use crate::args::{ConfirmArgs, ProposeArgs};

pub(crate) const PROPOSAL_TTL: TimeDelta = TimeDelta::seconds(120);
/// Demo defaults, matching `policy.yaml` `limits`; dispatch applies the
/// school's configured caller limits as well.
const PROPOSALS_PER_MINUTE: u32 = 5;
const CONFIRMS_PER_MINUTE: u32 = 2;
const LIVE_PER_USER: usize = 5;
const LIVE_GLOBAL: usize = 1000;
const BOOK_SCOPE: &str = "rooms:book";

const INVALID: &str = "This confirmation is not valid. Ask for a new proposal.";

#[derive(Debug)]
pub(crate) struct Booking {
    libcal: Arc<LibCal>,
    provider: SimulatedBooking,
    store: ProposalStore,
    proposals: RateLimiter,
    confirms: RateLimiter,
}

#[derive(Serialize)]
struct ProposedData<'a> {
    state: &'static str,
    proposal: ProposalView<'a>,
    confirmation_id: &'a str,
    expires_at: DateTime<Utc>,
    sandbox: bool,
    submitted: bool,
}

#[derive(Serialize)]
struct ProposalView<'a> {
    room_id: &'a str,
    location_id: &'a str,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
}

#[derive(Serialize)]
struct ConfirmedData<'a> {
    state: &'static str,
    receipt_id: &'a str,
    sandbox: bool,
    submitted: bool,
}

impl Booking {
    pub(crate) fn new(libcal: Arc<LibCal>) -> Self {
        Self {
            provider: SimulatedBooking::new(Arc::clone(&libcal)),
            libcal,
            store: ProposalStore::default(),
            proposals: RateLimiter::per_minute(PROPOSALS_PER_MINUTE, PROPOSALS_PER_MINUTE),
            confirms: RateLimiter::per_minute(CONFIRMS_PER_MINUTE, CONFIRMS_PER_MINUTE),
        }
    }

    /// `book_study_room`: check the slot and store a proposal. Read-only upstream.
    pub(crate) async fn propose(
        &self,
        args: Value,
        ctx: &CallContext,
    ) -> Result<ToolSuccess, ToolError> {
        self.proposals.check(&caller_key(ctx))?;
        require_student(ctx)?;
        let args: ProposeArgs = parse_args(args)?;
        args.check(ctx.now)?;
        let location_id = self
            .libcal
            .resolve_location(&args.location_id)
            .map_err(|e| {
                ToolError::invalid_arguments(format!(
                    "{} Use find_study_room for valid room_id and location_id values.",
                    e.message
                ))
            })?
            .to_owned();
        let slot = TimeSlot {
            start: args.start,
            end: args.end,
        };
        let free = self
            .libcal
            .is_free(&location_id, &args.room_id, &slot, Freshness::Cached)
            .await?;
        if !free.value {
            return Err(ToolError::invalid_arguments(
                "That room is not free for the whole requested time.",
            ));
        }
        let proposal = BookingProposal {
            confirmation_id: random_id("cnf_")?,
            school_id: ctx.school_id.clone(),
            subject: ctx.principal.subject.clone(),
            client_id: ctx.principal.client_id.clone(),
            device_id: ctx.principal.device_id.clone(),
            binding_generation: ctx.binding_generation,
            location_id,
            room_id: args.room_id,
            slot,
            expires_at: ctx.now + PROPOSAL_TTL,
        };
        let data = ProposedData {
            state: "proposed",
            proposal: ProposalView {
                room_id: &proposal.room_id,
                location_id: &proposal.location_id,
                start: proposal.slot.start,
                end: proposal.slot.end,
            },
            confirmation_id: &proposal.confirmation_id,
            expires_at: proposal.expires_at,
            sandbox: true,
            submitted: false,
        };
        let success = ToolSuccess::new(&data, sandbox_provenance(free.provenance))?;
        self.store.insert(
            Pending {
                proposal,
                policy_version: ctx.policy_version,
            },
            ctx.now,
        )?;
        Ok(success)
    }

    /// `confirm_booking`: reauthorize, consume the proposal once, revalidate
    /// availability freshly and return a sandbox receipt. Never writes upstream.
    pub(crate) async fn confirm(
        &self,
        args: Value,
        ctx: &CallContext,
    ) -> Result<ToolSuccess, ToolError> {
        self.confirms.check(&caller_key(ctx))?;
        require_student(ctx)?;
        if !ctx.principal.has_scope(BOOK_SCOPE) {
            return Err(ToolError::permission_denied(
                "Confirming a booking needs the rooms:book permission.",
            ));
        }
        let args: ConfirmArgs = parse_args(args)?;
        let pending = self.store.claim(&args.confirmation_id, ctx)?;
        match self
            .provider
            .confirm(&pending.proposal, &ctx.principal)
            .await
        {
            Ok(receipt) => {
                let data = ConfirmedData {
                    state: "confirmed",
                    receipt_id: &receipt.value.receipt_id,
                    sandbox: receipt.value.sandbox,
                    submitted: receipt.value.submitted,
                };
                ToolSuccess::new(&data, receipt.provenance)
            }
            Err(error) => {
                // A transient outage leaves the proposal usable until it expires.
                if error.retryable {
                    self.store.restore(pending);
                }
                Err(error)
            }
        }
    }
}

fn require_student(ctx: &CallContext) -> Result<(), ToolError> {
    if ctx.principal.has_role(Role::Student) {
        Ok(())
    } else {
        Err(ToolError::permission_denied(
            "Booking a room needs a signed-in student. Sign in, or tap your student card on the mascot.",
        ))
    }
}

/// In-memory limiter key; never persisted or logged.
fn caller_key(ctx: &CallContext) -> String {
    format!("{}\u{1f}{}", ctx.school_id, ctx.principal.subject)
}

fn sandbox_provenance(live: Provenance) -> Provenance {
    Provenance {
        source_kind: SourceKind::Sandbox,
        ..live
    }
}

#[derive(Debug, Clone)]
struct Pending {
    proposal: BookingProposal,
    policy_version: u64,
}

impl Pending {
    fn same_caller(&self, ctx: &CallContext) -> bool {
        let p = &self.proposal;
        p.school_id == ctx.school_id
            && p.subject == ctx.principal.subject
            && p.client_id == ctx.principal.client_id
            && p.device_id == ctx.principal.device_id
            && p.binding_generation == ctx.binding_generation
    }

    fn owner(&self) -> (&str, &str) {
        (&self.proposal.school_id, &self.proposal.subject)
    }
}

#[derive(Debug, Default)]
struct ProposalStore {
    live: Mutex<HashMap<String, Pending>>,
}

impl ProposalStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keep at most [`LIVE_PER_USER`] per caller (oldest replaced) and
    /// [`LIVE_GLOBAL`] overall.
    fn insert(&self, pending: Pending, now: DateTime<Utc>) -> Result<(), ToolError> {
        let mut live = self.lock();
        live.retain(|_, p| p.proposal.expires_at > now);
        let owner = pending.owner();
        let mut mine: Vec<(&String, DateTime<Utc>)> = live
            .iter()
            .filter(|(_, p)| p.owner() == owner)
            .map(|(id, p)| (id, p.proposal.expires_at))
            .collect();
        let evict = if mine.len() >= LIVE_PER_USER {
            mine.sort_by_key(|(_, expires)| *expires);
            mine.first().map(|(id, _)| (*id).clone())
        } else {
            None
        };
        if let Some(id) = evict {
            live.remove(&id);
        }
        if live.len() >= LIVE_GLOBAL {
            return Err(ToolError::rate_limited(60));
        }
        live.insert(pending.proposal.confirmation_id.clone(), pending);
        Ok(())
    }

    /// Atomically check and remove a proposal for this caller.
    ///
    /// Unknown, used and foreign IDs get the same answer, and the ID is never
    /// echoed. A foreign caller cannot burn someone else's proposal.
    fn claim(&self, id: &str, ctx: &CallContext) -> Result<Pending, ToolError> {
        let mut live = self.lock();
        let invalid = || ToolError::new(ErrorCode::ConfirmationInvalid, INVALID);
        let pending = live.get(id).ok_or_else(invalid)?;
        if !pending.same_caller(ctx) {
            return Err(invalid());
        }
        if pending.proposal.expires_at <= ctx.now {
            live.remove(id);
            return Err(ToolError::new(
                ErrorCode::ConfirmationExpired,
                "This proposal has expired. Ask for a new one.",
            ));
        }
        if pending.policy_version != ctx.policy_version {
            live.remove(id);
            return Err(invalid());
        }
        live.remove(id).ok_or_else(invalid)
    }

    /// Put back a claimed proposal after a transient failure.
    fn restore(&self, pending: Pending) {
        self.lock()
            .entry(pending.proposal.confirmation_id.clone())
            .or_insert(pending);
    }
}
