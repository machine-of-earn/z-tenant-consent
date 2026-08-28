//! The only place this crate talks to the host.
//!
//! Every host interface is reached through one small function here, for two
//! reasons: the capability surface stays readable in one screen, and the
//! native target gets honest stubs so `cargo test` can exercise the parsing
//! and decision paths without a cluster (see `model.rs`).

/// One tenant map holds the whole ledger: current consent rows (`c/…`),
/// superseded rows (`h/…`) and audit rows (`a/…`). One map means one ACL to
/// keep correct, which is the difference between a contract that survives a
/// redeploy and one that quietly loses read access.
pub const LEDGER_TAIL: &str = "ledger";
/// Provider URL and API key. Contract-only, seeded by the tenant SDK.
pub const SECRETS_TAIL: &str = "secrets";

pub const K_PROVIDER_URL: &[u8] = b"provider_url";
pub const K_PROVIDER_API_KEY: &[u8] = b"provider_api_key";
pub const K_PROVIDER_AUTH_HEADER: &[u8] = b"provider_auth_header";

#[cfg(target_arch = "wasm32")]
mod imp {
    use crate::host::interfaces::{http_with_placeholders as hwp, kv_store, logging};
    use crate::host::tenant::tenant_context;

    pub fn tenant_did_hex() -> String {
        hex::encode(tenant_context::tenant_did())
    }
    pub fn now_secs() -> u64 {
        tenant_context::cluster_timestamp_secs()
    }
    pub fn seq_no() -> u64 {
        tenant_context::seq_no()
    }
    pub fn contract_id() -> u32 {
        tenant_context::contract_id()
    }
    pub fn caller_did_hex() -> Option<String> {
        tenant_context::calling_user_did().map(hex::encode)
    }
    pub fn log_info(line: &str) {
        let _ = logging::info(line);
    }
    pub fn kv_get(map: &str, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        kv_store::get(map, key).map_err(|e| format!("kv get {map}: {e}"))
    }
    pub fn kv_put(map: &str, key: &[u8], value: &[u8]) -> Result<(), String> {
        kv_store::put(map, key, value).map_err(|e| format!("kv put {map}: {e}"))
    }
    pub fn kv_scan(
        map: &str,
        start: &[u8],
        end: &[u8],
        limit: u32,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        kv_store::scan(map, start, end, limit).map_err(|e| format!("kv scan {map}: {e}"))
    }

    /// POST through `http-with-placeholders`. The body contains
    /// `{{profile.*}}` markers the host resolves inside the enclave.
    pub fn post_with_placeholders(
        url: &str,
        headers: Vec<(String, String)>,
        payload: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), String> {
        let resp = hwp::call(&hwp::Request {
            method: hwp::Verb::Post,
            url: url.to_string(),
            headers: Some(headers),
            payload: Some(payload),
        })
        .map_err(format_http_error)?;
        Ok((resp.code, resp.payload))
    }

    /// Typed host errors are mapped to stable, operator-readable strings.
    /// None of them can carry resolved PII — that is the point of the variant.
    fn format_http_error(e: hwp::HttpError) -> String {
        match e {
            hwp::HttpError::EgressDenied(host) => format!(
                "egress_denied: host '{host}' is not on the caller's allowed-hosts grant"
            ),
            hwp::HttpError::PlaceholderDenied(marker) => {
                format!("placeholder_denied: this contract may not resolve {marker}")
            }
            hwp::HttpError::PlaceholderUnknown(field) => format!(
                "placeholder_unknown: the calling user's profile has no '{field}' — \
                 the notice was NOT sent"
            ),
            hwp::HttpError::PlaceholderNoUserContext => {
                "placeholder_no_user_context: this invocation has no user bound, so \
                 there is no profile to resolve the recipient from"
                    .to_string()
            }
            hwp::HttpError::UpstreamError(reason) => format!("upstream_error: {reason}"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    //! Native stubs. Host capabilities exist only inside the enclave, so on
    //! the native target every one of these fails loudly rather than
    //! pretending to work — unit tests cover the logic that runs *before*
    //! the first host call.
    const NO_HOST: &str = "host interface unavailable on the native target";

    pub fn tenant_did_hex() -> String {
        "00".repeat(20)
    }
    pub fn now_secs() -> u64 {
        0
    }
    pub fn seq_no() -> u64 {
        0
    }
    pub fn contract_id() -> u32 {
        0
    }
    pub fn caller_did_hex() -> Option<String> {
        None
    }
    pub fn log_info(_line: &str) {}
    pub fn kv_get(_map: &str, _key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        Err(NO_HOST.to_string())
    }
    pub fn kv_put(_map: &str, _key: &[u8], _value: &[u8]) -> Result<(), String> {
        Err(NO_HOST.to_string())
    }
    pub fn kv_scan(
        _map: &str,
        _start: &[u8],
        _end: &[u8],
        _limit: u32,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        Err(NO_HOST.to_string())
    }
    pub fn post_with_placeholders(
        _url: &str,
        _headers: Vec<(String, String)>,
        _payload: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), String> {
        Err(NO_HOST.to_string())
    }
}

pub use imp::*;

/// `z:<tid>:<tail>` — `kv-store` takes the full canonical name, and the host
/// enforces the prefix. Built at runtime from `tenant-context`, never
/// hardcoded.
pub fn map_name(tail: &str) -> String {
    format!("z:{}:{}", tenant_did_hex(), tail)
}
