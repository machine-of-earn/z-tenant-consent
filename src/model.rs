//! Pure data model and decision logic.
//!
//! Everything in this file compiles and runs on the NATIVE target, with no
//! host imports, so the rules that matter — when a send is allowed, and what
//! a caller is allowed to put in a notice — are covered by `cargo test`
//! without a cluster, a network, or a WASM runtime. That is deliberate: the
//! host-touching wrappers in `host_shim.rs` are a thin layer over this.

use serde::{Deserialize, Serialize};

/// A consent grant for one subject and one notice category.
///
/// `subject_ref` is an opaque tenant-side customer reference (e.g. a CRM id).
/// It is never an email address, a name or any other contact datum — see
/// [`reject_contact_data`], which is applied to it on the way in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsentRow {
    pub subject_ref: String,
    pub category: String,
    pub granted: bool,
    /// Unix seconds after which the grant is stale. `None` = no expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Free-text pointer to where the consent was captured (a form id, a
    /// ticket number). Not evidence itself — a pointer to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_ref: Option<String>,
    pub recorded_at: u64,
    pub seq_no: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    NoConsentOnRecord,
    Withdrawn,
    Expired,
}

impl Decision {
    pub fn allowed(&self) -> bool {
        matches!(self, Decision::Allowed)
    }
    /// A short, stable, machine-readable reason. Stable strings matter: they
    /// end up in the audit ledger and in the caller's error handling.
    pub fn reason(&self) -> &'static str {
        match self {
            Decision::Allowed => "consent_valid",
            Decision::NoConsentOnRecord => "no_consent_on_record",
            Decision::Withdrawn => "consent_withdrawn",
            Decision::Expired => "consent_expired",
        }
    }
}

/// The gate. Fails closed: anything other than a present, granted, unexpired
/// row is a refusal.
pub fn decide(row: Option<&ConsentRow>, now_secs: u64) -> Decision {
    match row {
        None => Decision::NoConsentOnRecord,
        Some(r) if !r.granted => Decision::Withdrawn,
        Some(r) => match r.expires_at {
            // Expiry is inclusive of the boundary second: a grant that
            // expires_at == now is already expired.
            Some(exp) if now_secs >= exp => Decision::Expired,
            _ => Decision::Allowed,
        },
    }
}

/// Substrings that mark a field as contact data or PII. Matched
/// case-insensitively against parameter KEYS.
const PII_KEY_MARKERS: &[&str] = &[
    "email", "mail", "phone", "tel", "mobile", "msisdn", "address", "addr",
    "street", "postcode", "zipcode", "zip", "dob", "birth", "ssn", "passport",
    "nric", "iban", "card", "given_name", "family_name", "first_name",
    "last_name", "fullname", "full_name", "surname", "recipient", "to",
];

/// Reject any string that carries contact data.
///
/// The contract's whole promise is that the recipient is supplied by the host
/// from the calling user's profile, never by the caller. That promise is only
/// worth something if the caller cannot smuggle a recipient in through a
/// template parameter, so every caller-supplied key and value is screened
/// here before anything is written or sent.
pub fn reject_contact_data(field: &str, key: &str, value: &str) -> Result<(), String> {
    let k = key.to_ascii_lowercase();
    for marker in PII_KEY_MARKERS {
        if k == *marker || k.contains(marker) {
            return Err(format!(
                "bad input: {field} key '{key}' looks like contact data ('{marker}'); \
                 the recipient is resolved host-side from the user profile and must \
                 never be passed in"
            ));
        }
    }
    if looks_like_email(value) {
        return Err(format!(
            "bad input: {field} value under '{key}' looks like an email address; \
             the recipient is resolved host-side from the user profile"
        ));
    }
    Ok(())
}

/// True when the string contains an address-shaped token *anywhere* — an
/// address in the middle of prose ("reply to jane@example.com") counts, which
/// is the case a naive whole-string check misses.
pub fn looks_like_email(value: &str) -> bool {
    value
        .split(|c: char| c.is_whitespace() || ",;:<>()[]{}\"'\\".contains(c))
        .any(is_email_token)
}

/// One token: a non-empty local part, exactly one `@`, and a domain with a
/// dot that is neither first nor last. Deliberately conservative — it errs
/// towards refusing.
fn is_email_token(token: &str) -> bool {
    let token = token.trim_end_matches(['.', '!', '?']);
    let mut parts = token.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty() || domain.len() < 3 {
        return false;
    }
    match domain.find('.') {
        Some(dot) => dot > 0 && dot < domain.len() - 1,
        None => false,
    }
}

/// Opaque references must be safe to put in a KV key: no `/` (the key
/// separator), no control characters, bounded length, and not PII.
pub fn validate_ref(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("bad input: {field} must not be empty"));
    }
    if value.len() > 128 {
        return Err(format!("bad input: {field} must be 128 characters or fewer"));
    }
    if value.contains('/') {
        return Err(format!("bad input: {field} must not contain '/'"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("bad input: {field} must not contain control characters"));
    }
    if looks_like_email(value) {
        return Err(format!(
            "bad input: {field} looks like an email address — use an opaque \
             customer reference, never a contact detail"
        ));
    }
    Ok(())
}

/// `c/<subject>/<category>` — the current consent row.
pub fn consent_key(subject_ref: &str, category: &str) -> Vec<u8> {
    format!("c/{subject_ref}/{category}").into_bytes()
}

/// `h/<subject>/<category>/<seq zero-padded>` — superseded rows, kept forever.
/// Zero padding keeps `scan`'s lexicographic order equal to sequence order.
pub fn history_key(subject_ref: &str, category: &str, seq_no: u64) -> Vec<u8> {
    format!("h/{subject_ref}/{category}/{seq_no:020}").into_bytes()
}

/// `a/<subject>/<seq zero-padded>` — one append-only audit row per decision.
pub fn audit_key(subject_ref: &str, seq_no: u64) -> Vec<u8> {
    format!("a/{subject_ref}/{seq_no:020}").into_bytes()
}

/// Half-open `[start, end)` bounds for scanning one subject's audit rows, or
/// every subject's when `subject_ref` is `None`.
pub fn audit_scan_range(subject_ref: Option<&str>) -> (Vec<u8>, Vec<u8>) {
    match subject_ref {
        // `0` is the byte right after `/`, so `a/<s>0` is the exclusive
        // successor of every `a/<s>/...` key — and it also excludes a
        // neighbouring subject like `cust-10` when scanning `cust-1`.
        Some(s) => (
            format!("a/{s}/").into_bytes(),
            format!("a/{s}0").into_bytes(),
        ),
        None => (b"a/".to_vec(), b"a0".to_vec()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub seq_no: u64,
    pub at: u64,
    pub subject_ref: String,
    pub category: String,
    pub template_id: String,
    /// "dispatched" | "refused" | "provider_error"
    pub outcome: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u16>,
    pub contract_id: u32,
    /// Hex of the calling user's DID when the call came through the session
    /// API; `None` for direct `/api/dev/exec` invocations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_did: Option<String>,
}

/// The marker the host resolves, inside the enclave, into the calling user's
/// verified email address. This literal string is the only "recipient" this
/// contract ever holds.
pub const RECIPIENT_PLACEHOLDER: &str = "{{profile.verified_contacts.email.value}}";

/// Build the provider request body. Provider-agnostic on purpose: it is a
/// flat JSON envelope any transactional-email API can be pointed at with a
/// small mapping, and it carries the consent basis alongside the send so the
/// provider's own logs are auditable too.
pub fn build_notice_body(
    template_id: &str,
    params: &serde_json::Value,
    row: &ConsentRow,
    idempotency_key: &str,
) -> serde_json::Value {
    serde_json::json!({
        "to": RECIPIENT_PLACEHOLDER,
        "template_id": template_id,
        "params": params,
        "consent": {
            "subject_ref": row.subject_ref,
            "category": row.category,
            "recorded_at": row.recorded_at,
            "expires_at": row.expires_at,
            "evidence_ref": row.evidence_ref,
        },
        "idempotency_key": idempotency_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(granted: bool, expires_at: Option<u64>) -> ConsentRow {
        ConsentRow {
            subject_ref: "cust-1".into(),
            category: "billing".into(),
            granted,
            expires_at,
            evidence_ref: None,
            recorded_at: 1000,
            seq_no: 7,
        }
    }

    #[test]
    fn absent_consent_is_refused() {
        assert_eq!(decide(None, 2000), Decision::NoConsentOnRecord);
        assert!(!decide(None, 2000).allowed());
    }

    #[test]
    fn withdrawn_consent_is_refused() {
        assert_eq!(decide(Some(&row(false, None)), 2000), Decision::Withdrawn);
    }

    #[test]
    fn expired_consent_is_refused_including_on_the_boundary_second() {
        assert_eq!(decide(Some(&row(true, Some(2000))), 2000), Decision::Expired);
        assert_eq!(decide(Some(&row(true, Some(2000))), 2001), Decision::Expired);
        assert_eq!(decide(Some(&row(true, Some(2001))), 2000), Decision::Allowed);
    }

    #[test]
    fn granted_unexpired_consent_is_allowed() {
        assert!(decide(Some(&row(true, None)), 99_999).allowed());
    }

    #[test]
    fn params_carrying_a_recipient_are_rejected() {
        for key in ["email", "customer_email", "to", "phone_number", "billing_address", "date_of_birth", "first_name"] {
            assert!(
                reject_contact_data("params", key, "x").is_err(),
                "key {key} should have been rejected"
            );
        }
    }

    #[test]
    fn a_recipient_hidden_in_a_harmless_key_is_still_rejected() {
        let err = reject_contact_data("params", "ref", "jane.doe@example.com").unwrap_err();
        assert!(err.contains("email address"), "{err}");
    }

    #[test]
    fn ordinary_template_params_pass() {
        assert!(reject_contact_data("params", "invoice_no", "INV-2026-0042").is_ok());
        assert!(reject_contact_data("params", "amount_due", "129.00").is_ok());
    }

    #[test]
    fn email_detector_does_not_fire_on_ordinary_text() {
        assert!(!looks_like_email("INV-2026-0042"));
        assert!(!looks_like_email("@handle"));
        assert!(!looks_like_email("a @ b.com"));
        assert!(!looks_like_email("user@localhost"));
        assert!(looks_like_email("jane@example.com"));
        // The case a whole-string check misses: an address inside prose.
        assert!(looks_like_email("reply to jane.doe@example.com if wrong"));
        assert!(looks_like_email("<jane@example.co.uk>"));
    }

    #[test]
    fn refs_must_be_opaque_and_key_safe() {
        assert!(validate_ref("subject_ref", "cust-1").is_ok());
        assert!(validate_ref("subject_ref", "").is_err());
        assert!(validate_ref("subject_ref", "a/b").is_err());
        assert!(validate_ref("subject_ref", "jane@example.com").is_err());
        assert!(validate_ref("subject_ref", &"x".repeat(129)).is_err());
    }

    #[test]
    fn history_and_audit_keys_sort_by_sequence() {
        let mut keys = vec![
            history_key("c", "billing", 10),
            history_key("c", "billing", 2),
            history_key("c", "billing", 100),
        ];
        keys.sort();
        assert_eq!(keys[0], history_key("c", "billing", 2));
        assert_eq!(keys[2], history_key("c", "billing", 100));
    }

    #[test]
    fn audit_scan_range_is_half_open_and_subject_scoped() {
        let (start, end) = audit_scan_range(Some("cust-1"));
        let k = audit_key("cust-1", 5);
        assert!(k >= start && k < end);
        // A different subject must fall outside the range — including the
        // prefix-collision case, which a naive bound gets wrong.
        for other in [audit_key("cust-2", 5), audit_key("cust-10", 5)] {
            assert!(!(other >= start && other < end), "{}", String::from_utf8_lossy(&other));
        }
    }

    #[test]
    fn the_body_carries_a_marker_and_never_a_real_recipient() {
        let r = row(true, None);
        let body = build_notice_body("invoice_due", &serde_json::json!({"invoice_no": "INV-1"}), &r, "idem-1");
        assert_eq!(body["to"], RECIPIENT_PLACEHOLDER);
        let s = serde_json::to_string(&body).unwrap();
        assert!(!s.contains('@') || s.contains("{{profile."), "{s}");
        assert!(s.contains("\"subject_ref\":\"cust-1\""));
    }
}
