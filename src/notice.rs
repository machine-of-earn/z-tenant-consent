//! send-notice — the gated send.
//!
//! Order of operations is the whole design:
//!   1. parse and screen the caller's input (no host call yet),
//!   2. read the consent row and decide,
//!   3. on a refusal, write the audit row and RETURN — no egress happens,
//!   4. only then resolve the provider and make one outbound call whose
//!      recipient is a `{{profile.*}}` marker,
//!   5. write the audit row for the send.
//!
//! Steps 3 and 4 are in that order deliberately: a refused notice must cost
//! nothing and leak nothing, not even the existence of the subject to the
//! provider.

use crate::audit;
use crate::consent::load_row;
use crate::host_shim as host;
use crate::model::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct SendReq {
    subject_ref: String,
    category: String,
    template_id: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// Screen caller-supplied template parameters.
///
/// Only a flat object of scalars is accepted. Nesting is refused rather than
/// walked: a recursive screen is a place for a bypass to hide, and no
/// transactional template needs one.
pub fn screen_params(params: &serde_json::Value) -> Result<(), String> {
    let obj = params
        .as_object()
        .ok_or("bad input: params must be a JSON object")?;
    if obj.len() > 32 {
        return Err("bad input: params may hold at most 32 entries".to_string());
    }
    for (key, value) in obj {
        let scalar = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => String::new(),
            _ => {
                return Err(format!(
                    "bad input: params['{key}'] must be a string, number, boolean or null — \
                     nested objects and arrays are not accepted"
                ))
            }
        };
        if scalar.len() > 512 {
            return Err(format!("bad input: params['{key}'] exceeds 512 characters"));
        }
        reject_contact_data("params", key, &scalar)?;
    }
    Ok(())
}

struct Provider {
    url: String,
    headers: Vec<(String, String)>,
}

/// Resolve the provider from the contract-only `secrets` map. Read at call
/// time rather than baked into the WASM, so rotating a key or repointing the
/// provider is a control-plane write, not a redeploy.
fn load_provider() -> Result<Provider, String> {
    let secrets = host::map_name(host::SECRETS_TAIL);
    let url = host::kv_get(&secrets, host::K_PROVIDER_URL)?
        .ok_or("provider_url not found in the secrets map — seed it with map-entry-set before use")?;
    let url = String::from_utf8(url).map_err(|e| format!("provider_url is not utf-8: {e}"))?;

    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(key) = host::kv_get(&secrets, host::K_PROVIDER_API_KEY)? {
        let key = String::from_utf8(key).map_err(|e| format!("provider_api_key is not utf-8: {e}"))?;
        let header_name = match host::kv_get(&secrets, host::K_PROVIDER_AUTH_HEADER)? {
            Some(h) => String::from_utf8(h)
                .map_err(|e| format!("provider_auth_header is not utf-8: {e}"))?,
            None => "Authorization".to_string(),
        };
        let value = if header_name.eq_ignore_ascii_case("Authorization") {
            format!("Bearer {key}")
        } else {
            key
        };
        headers.push((header_name, value));
    }
    Ok(Provider { url, headers })
}

pub fn send_notice(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: SendReq = serde_json::from_slice(input).map_err(|e| format!("bad input: {e}"))?;
    validate_ref("subject_ref", &req.subject_ref)?;
    validate_ref("category", &req.category)?;
    validate_ref("template_id", &req.template_id)?;
    let params = req.params.unwrap_or_else(|| serde_json::json!({}));
    screen_params(&params)?;

    let now = host::now_secs();
    let seq = host::seq_no();
    let row = load_row(&req.subject_ref, &req.category)?;
    let decision = decide(row.as_ref(), now);

    if !decision.allowed() {
        // Refusal path: nothing is dispatched, and no request is built.
        let key = audit::write_row(AuditRow {
            seq_no: seq,
            at: now,
            subject_ref: req.subject_ref.clone(),
            category: req.category.clone(),
            template_id: req.template_id.clone(),
            outcome: "refused".to_string(),
            reason: decision.reason().to_string(),
            provider_status: None,
            contract_id: host::contract_id(),
            caller_did: host::caller_did_hex(),
        })?;
        host::log_info("notice refused at the consent gate");
        return serde_json::to_vec(&serde_json::json!({
            "dispatched": false,
            "reason": decision.reason(),
            "audit_key": key,
            "at": now,
        }))
        .map_err(|e| e.to_string());
    }

    let row = row.expect("decide() only allows a present row");
    let provider = load_provider()?;
    let idempotency_key = format!(
        "{}:{}:{}:{}",
        host::tenant_did_hex(),
        req.subject_ref,
        req.category,
        seq
    );
    let body = build_notice_body(&req.template_id, &params, &row, &idempotency_key);
    let payload = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

    let (code, resp) = host::post_with_placeholders(&provider.url, provider.headers, payload)?;
    let ok = (200..300).contains(&code);

    let key = audit::write_row(AuditRow {
        seq_no: seq,
        at: now,
        subject_ref: req.subject_ref.clone(),
        category: req.category.clone(),
        template_id: req.template_id.clone(),
        outcome: if ok { "dispatched" } else { "provider_error" }.to_string(),
        reason: if ok {
            decision.reason().to_string()
        } else {
            format!("provider returned HTTP {code}")
        },
        provider_status: Some(code),
        contract_id: host::contract_id(),
        caller_did: host::caller_did_hex(),
    })?;
    host::log_info(if ok { "notice dispatched" } else { "provider rejected the notice" });

    serde_json::to_vec(&serde_json::json!({
        "dispatched": ok,
        "reason": if ok { decision.reason().to_string() } else { format!("provider returned HTTP {code}") },
        "provider_status": code,
        // Bounded so a chatty provider cannot blow up the response, and it is
        // the provider's own echo — the recipient in it was resolved host-side.
        "provider_body": String::from_utf8_lossy(&resp[..resp.len().min(2048)]),
        "audit_key": key,
        "idempotency_key": idempotency_key,
        "at": now,
    }))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(v: serde_json::Value) -> Result<Vec<u8>, String> {
        send_notice(&serde_json::to_vec(&v).unwrap())
    }

    #[test]
    fn a_recipient_in_params_is_refused_before_any_host_call() {
        let err = send(serde_json::json!({
            "subject_ref": "cust-1", "category": "billing", "template_id": "invoice_due",
            "params": { "email": "jane@example.com" }
        }))
        .unwrap_err();
        assert!(err.contains("contact data"), "{err}");
        // Proves it failed at the screen, not at the host: a host failure
        // would say "host interface unavailable".
        assert!(!err.contains("host interface"), "{err}");
    }

    #[test]
    fn a_recipient_hidden_under_an_innocent_key_is_refused_too() {
        let err = send(serde_json::json!({
            "subject_ref": "cust-1", "category": "billing", "template_id": "invoice_due",
            "params": { "note": "reply to jane@example.com" }
        }))
        .unwrap_err();
        assert!(err.contains("email address"), "{err}");
    }

    #[test]
    fn nested_params_are_refused_rather_than_walked() {
        let err = send(serde_json::json!({
            "subject_ref": "cust-1", "category": "billing", "template_id": "invoice_due",
            "params": { "customer": { "email": "jane@example.com" } }
        }))
        .unwrap_err();
        assert!(err.contains("nested objects"), "{err}");
    }

    #[test]
    fn clean_params_get_past_the_screen_and_stop_at_the_host_boundary() {
        // Natively there is no KV store, so the furthest this can get is the
        // consent read — which is exactly the assertion: screening passed.
        let err = send(serde_json::json!({
            "subject_ref": "cust-1", "category": "billing", "template_id": "invoice_due",
            "params": { "invoice_no": "INV-2026-0042", "amount_due": 129.0, "final": true }
        }))
        .unwrap_err();
        assert!(err.contains("host interface unavailable"), "{err}");
    }

    #[test]
    fn a_missing_template_id_is_refused() {
        let err = send(serde_json::json!({ "subject_ref": "cust-1", "category": "billing" }))
            .unwrap_err();
        assert!(err.starts_with("bad input"), "{err}");
    }

    #[test]
    fn params_must_be_an_object() {
        assert!(screen_params(&serde_json::json!(["a"]))
            .unwrap_err()
            .contains("must be a JSON object"));
    }
}
