//! z-tenant-consent — consent-gated notice dispatch as a T3N TEE contract.
//!
//! See `wit/world.wit` for the exported interface and the exact set of host
//! capabilities this contract holds. The short version:
//!
//!   * a notice is only ever sent when a granted, unexpired consent row
//!     exists for that subject and that category — the gate fails closed,
//!   * the recipient is never held by this contract: it templates
//!     `{{profile.verified_contacts.email.value}}` into the provider request
//!     and the host resolves it inside the enclave,
//!   * every decision, sent or refused, appends a row to a tamper-evident
//!     ledger that an auditor can read back with `audit-log`.

wit_bindgen::generate!({
    world: "tenant-consent",
    path: "wit",
    additional_derives: [serde::Deserialize, serde::Serialize],
    generate_all,
});

pub mod audit;
pub mod consent;
pub mod host_shim;
pub mod model;
pub mod notice;

struct Component;

#[cfg(target_arch = "wasm32")]
impl exports::z::tenant_consent::contracts::Guest for Component {
    fn record_consent(
        req: exports::z::tenant_consent::contracts::GenericInput,
    ) -> Result<Vec<u8>, String> {
        consent::record_consent(&req.input.ok_or("record-consent: missing input")?)
    }

    fn withdraw_consent(
        req: exports::z::tenant_consent::contracts::GenericInput,
    ) -> Result<Vec<u8>, String> {
        consent::withdraw_consent(&req.input.ok_or("withdraw-consent: missing input")?)
    }

    fn check_consent(
        req: exports::z::tenant_consent::contracts::GenericInput,
    ) -> Result<Vec<u8>, String> {
        consent::check_consent(&req.input.ok_or("check-consent: missing input")?)
    }

    fn send_notice(
        req: exports::z::tenant_consent::contracts::GenericInput,
    ) -> Result<Vec<u8>, String> {
        notice::send_notice(&req.input.ok_or("send-notice: missing input")?)
    }

    fn audit_log(
        req: exports::z::tenant_consent::contracts::GenericInput,
    ) -> Result<Vec<u8>, String> {
        // audit-log takes no required argument, so an absent input is a
        // valid "everything, default limit" call rather than an error.
        audit::audit_log(&req.input.unwrap_or_default())
    }
}

#[cfg(target_arch = "wasm32")]
export!(Component);
