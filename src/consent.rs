//! record-consent / withdraw-consent / check-consent.

use crate::host_shim as host;
use crate::model::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct RecordReq {
    subject_ref: String,
    category: String,
    #[serde(default = "yes")]
    granted: bool,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    evidence_ref: Option<String>,
}
fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct SubjectCategoryReq {
    subject_ref: String,
    category: String,
    #[serde(default)]
    evidence_ref: Option<String>,
}

fn parse<'a, T: Deserialize<'a>>(input: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(input).map_err(|e| format!("bad input: {e}"))
}

fn check_refs(subject_ref: &str, category: &str) -> Result<(), String> {
    validate_ref("subject_ref", subject_ref)?;
    validate_ref("category", category)
}

/// Write the current row aside into history before it is replaced, so the
/// ledger keeps every state a subject's consent has ever been in.
fn archive_current(ledger: &str, subject_ref: &str, category: &str) -> Result<bool, String> {
    let key = consent_key(subject_ref, category);
    match host::kv_get(ledger, &key)? {
        None => Ok(false),
        Some(bytes) => {
            let prior: ConsentRow = serde_json::from_slice(&bytes)
                .map_err(|e| format!("corrupt consent row for {subject_ref}/{category}: {e}"))?;
            host::kv_put(
                ledger,
                &history_key(subject_ref, category, prior.seq_no),
                &bytes,
            )?;
            Ok(true)
        }
    }
}

fn store(row: &ConsentRow) -> Result<Vec<u8>, String> {
    let ledger = host::map_name(host::LEDGER_TAIL);
    let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
    host::kv_put(&ledger, &consent_key(&row.subject_ref, &row.category), &bytes)?;
    Ok(bytes)
}

pub fn record_consent(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: RecordReq = parse(input)?;
    check_refs(&req.subject_ref, &req.category)?;
    if let Some(ref e) = req.evidence_ref {
        validate_ref("evidence_ref", e)?;
    }

    let ledger = host::map_name(host::LEDGER_TAIL);
    let had_prior = archive_current(&ledger, &req.subject_ref, &req.category)?;

    let row = ConsentRow {
        subject_ref: req.subject_ref,
        category: req.category,
        granted: req.granted,
        expires_at: req.expires_at,
        evidence_ref: req.evidence_ref,
        recorded_at: host::now_secs(),
        seq_no: host::seq_no(),
    };
    store(&row)?;
    host::log_info("consent recorded");

    serde_json::to_vec(&serde_json::json!({ "row": row, "superseded_prior": had_prior }))
        .map_err(|e| e.to_string())
}

pub fn withdraw_consent(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: SubjectCategoryReq = parse(input)?;
    check_refs(&req.subject_ref, &req.category)?;
    if let Some(ref e) = req.evidence_ref {
        validate_ref("evidence_ref", e)?;
    }

    let ledger = host::map_name(host::LEDGER_TAIL);
    // A withdrawal is recorded even when nothing was on record. An auditor
    // asking "did this customer ever say stop?" must get an answer either way.
    let had_prior = archive_current(&ledger, &req.subject_ref, &req.category)?;

    let row = ConsentRow {
        subject_ref: req.subject_ref,
        category: req.category,
        granted: false,
        expires_at: None,
        evidence_ref: req.evidence_ref,
        recorded_at: host::now_secs(),
        seq_no: host::seq_no(),
    };
    store(&row)?;
    host::log_info("consent withdrawn");

    serde_json::to_vec(&serde_json::json!({ "row": row, "had_prior_consent": had_prior }))
        .map_err(|e| e.to_string())
}

/// Read the current row. Shared with `send-notice` so the gate and the
/// read-only check can never drift apart.
pub fn load_row(subject_ref: &str, category: &str) -> Result<Option<ConsentRow>, String> {
    let ledger = host::map_name(host::LEDGER_TAIL);
    match host::kv_get(&ledger, &consent_key(subject_ref, category))? {
        None => Ok(None),
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("corrupt consent row for {subject_ref}/{category}: {e}")),
    }
}

pub fn check_consent(input: &[u8]) -> Result<Vec<u8>, String> {
    #[derive(Deserialize)]
    struct Req {
        subject_ref: String,
        category: String,
    }
    let req: Req = parse(input)?;
    check_refs(&req.subject_ref, &req.category)?;

    let now = host::now_secs();
    let row = load_row(&req.subject_ref, &req.category)?;
    let decision = decide(row.as_ref(), now);

    serde_json::to_vec(&serde_json::json!({
        "allowed": decision.allowed(),
        "reason": decision.reason(),
        "checked_at": now,
        "row": row,
    }))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_consent_rejects_a_contact_detail_as_the_subject_ref() {
        let input = serde_json::to_vec(&serde_json::json!({
            "subject_ref": "jane@example.com", "category": "billing"
        }))
        .unwrap();
        let err = record_consent(&input).unwrap_err();
        assert!(err.contains("opaque customer reference"), "{err}");
    }

    #[test]
    fn record_consent_rejects_a_category_with_a_key_separator() {
        let input = serde_json::to_vec(&serde_json::json!({
            "subject_ref": "cust-1", "category": "billing/urgent"
        }))
        .unwrap();
        assert!(record_consent(&input).unwrap_err().contains("must not contain '/'"));
    }

    #[test]
    fn record_consent_rejects_non_json() {
        assert!(record_consent(b"not json").unwrap_err().starts_with("bad input"));
    }

    #[test]
    fn check_consent_rejects_a_missing_field_before_touching_the_host() {
        let input = serde_json::to_vec(&serde_json::json!({ "subject_ref": "cust-1" })).unwrap();
        assert!(check_consent(&input).unwrap_err().starts_with("bad input"));
    }
}
