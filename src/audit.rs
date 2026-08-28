//! audit-log — read the append-only decision ledger.

use crate::host_shim as host;
use crate::model::*;
use serde::Deserialize;

/// Default and ceiling for a single scan. `kv-store::scan` is one-shot with
/// no cursor, so a caller walking a large ledger re-calls with a narrower
/// range rather than paging.
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;

/// Append one audit row and return its key. Rows are never updated or
/// deleted: `seq_no` comes from the host's store sequence, so two rows can
/// not collide and the key order is the decision order.
pub fn write_row(row: AuditRow) -> Result<String, String> {
    let ledger = host::map_name(host::LEDGER_TAIL);
    let key = audit_key(&row.subject_ref, row.seq_no);
    let bytes = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
    host::kv_put(&ledger, &key, &bytes)?;
    Ok(String::from_utf8_lossy(&key).to_string())
}

pub fn audit_log(input: &[u8]) -> Result<Vec<u8>, String> {
    #[derive(Deserialize, Default)]
    struct Req {
        #[serde(default)]
        subject_ref: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
    }
    let req: Req = if input.is_empty() {
        Req::default()
    } else {
        serde_json::from_slice(input).map_err(|e| format!("bad input: {e}"))?
    };
    if let Some(ref s) = req.subject_ref {
        validate_ref("subject_ref", s)?;
    }
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let ledger = host::map_name(host::LEDGER_TAIL);
    let (start, end) = audit_scan_range(req.subject_ref.as_deref());
    let pairs = host::kv_scan(&ledger, &start, &end, limit)?;

    let mut rows = Vec::with_capacity(pairs.len());
    for (key, value) in &pairs {
        let row: AuditRow = serde_json::from_slice(value)
            .map_err(|e| format!("corrupt audit row {}: {e}", String::from_utf8_lossy(key)))?;
        rows.push(row);
    }

    serde_json::to_vec(&serde_json::json!({
        "rows": rows,
        "count": rows.len(),
        "limit": limit,
        // True when the scan filled its budget: the caller should narrow the
        // range and re-call rather than assume it saw everything.
        "truncated": rows.len() as u32 == limit,
    }))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_email_as_the_subject_filter_is_refused() {
        let input = serde_json::to_vec(&serde_json::json!({ "subject_ref": "jane@example.com" })).unwrap();
        assert!(audit_log(&input).unwrap_err().contains("opaque customer reference"));
    }

    #[test]
    fn a_bad_limit_is_clamped_rather_than_rejected() {
        // Reaches the host (which is absent natively) instead of erroring on
        // the limit — proving the clamp ran.
        let input = serde_json::to_vec(&serde_json::json!({ "limit": 100000 })).unwrap();
        assert!(audit_log(&input).unwrap_err().contains("host interface unavailable"));
    }
}
