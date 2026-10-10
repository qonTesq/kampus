//! Sandbox booking simulation. Compiled only with the `sandbox` feature and
//! built only when `sandbox.enabled` and `libcal.booking_simulation` are set.
//! It revalidates availability with a fresh public read and issues a
//! synthetic receipt; it never sends anything to a `LibCal` write endpoint.

use std::fmt::Write as _;
use std::sync::Arc;

use kampus_core::contracts::providers::{
    BookingProposal, BookingProvider, BookingReceipt, ProviderResult, Sourced,
};
use kampus_core::contracts::{BoxFuture, ErrorCode, Principal, Provenance, SourceKind, ToolError};

use crate::{Freshness, LibCal};

#[derive(Debug)]
pub struct SimulatedBooking {
    libcal: Arc<LibCal>,
}

impl SimulatedBooking {
    pub fn new(libcal: Arc<LibCal>) -> Self {
        Self { libcal }
    }
}

impl BookingProvider for SimulatedBooking {
    fn confirm<'a>(
        &'a self,
        proposal: &'a BookingProposal,
        _principal: &'a Principal,
    ) -> BoxFuture<'a, ProviderResult<BookingReceipt>> {
        Box::pin(async move {
            let free = self
                .libcal
                .is_free(
                    &proposal.location_id,
                    &proposal.room_id,
                    &proposal.slot,
                    Freshness::Fresh,
                )
                .await?;
            if !free.value {
                return Err(ToolError::new(
                    ErrorCode::ConfirmationInvalid,
                    "The room is no longer free for that time. Ask for a new proposal.",
                ));
            }
            Ok(Sourced {
                value: BookingReceipt {
                    receipt_id: random_id("sbx_")?,
                    sandbox: true,
                    submitted: false,
                },
                provenance: Provenance {
                    source_kind: SourceKind::Sandbox,
                    ..free.provenance
                },
            })
        })
    }
}

/// `prefix` plus 128 random bits from the OS CSPRNG, hex encoded.
pub fn random_id(prefix: &str) -> Result<String, ToolError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| ToolError::configuration_error())?;
    let mut id = String::with_capacity(prefix.len() + 32);
    id.push_str(prefix);
    for b in bytes {
        let _ = write!(id, "{b:02x}");
    }
    Ok(id)
}
