# Bug log — T3N ADK, as found building `z-tenant-consent`

Every entry below was hit while building this contract on **2026-08-28**, against
`testnet`, with `@terminal3/t3n-sdk@5.2.0`, `wit-bindgen 0.49`, `cargo 1.98.0`,
target `wasm32-wasip2`, Node v22.14.0. Each one has a reproduction and the exact
text the tool produced. Nothing here is inferred.

Tenant DID used throughout: `did:t3n:9e0cfe5b257503525ad98c65e6ab7d7f09fbf620`.
Contract: `z:9e0cfe5b257503525ad98c65e6ab7d7f09fbf620:consent`, contract id `778`.

---

## 1. Undocumented per-minute fuel quota — `quota exceeded (fuel_per_minute)`

**Severity: high (it stops a working integration with no warning in the docs).**

Running the end-to-end demo twice inside the same minute fails partway through:

```
RpcError: RPC Error: quota exceeded (fuel_per_minute): tenant 9e0cfe...f620
  on contract z:9e0cfe...f620:consent [197bc9b7-5d36-4aa1-a68a-e3c6a7113bae]
  code: 'RPC_ERROR', rpcMethod: 'action.execute', httpStatus: -32002
```

**Repro:** `npx tsx ops/demo.ts` twice in a row (13 contract calls each, one of
them an outbound HTTP call). The second run dies at step 7.

**Why it matters:** `fuel_per_minute` does not appear anywhere in the docs —
not in [Tokens](https://docs.terminal3.io/t3n/how-t3n-works/tokens),
not in [Common errors](https://docs.terminal3.io/developers/adk/tips/common-errors),
not in the reference. The docs describe credits as a *balance* that depletes
(`InsufficientCreditError`), which reads as "you have N calls total", not "you
have N units per minute". A developer whose first integration test loop trips
this has no way to tell a rate limit from an exhausted account.

**Ask:** document the limit, its window, and its reset — and ideally return a
`retry_after`. Waiting ~100 seconds and re-running succeeded, so the window is
short; the exact figure is the sponsor's to state, and this log deliberately
does not guess it.

---

## 2. The placeholders tip page shows Rust that does not compile

**Severity: medium (copy-paste from the docs fails at the first build).**

[Placeholders in outbound calls](https://docs.terminal3.io/developers/adk/tips/placeholders-outbound-calls)
shows the request built like this:

```rust
let resp = hwp::call(&hwp::Request {
    method:  "POST".to_string(),
    url:     "https://api.duffel.com/air/orders".to_string(),
    headers: vec![ ("Authorization".to_string(), format!("Bearer {api_key}")) ],
    payload: Some(serde_json::to_vec(&body)?),
})?;
```

The vendored WIT (`wit/deps/host-interfaces-2.1.0/package.wit`, the same copy
shipped in `Terminal-3/z-tenant-flight`) declares:

```wit
record request {
  method: verb,                                    // enum, not a string
  url: string,
  headers: option<list<tuple<string, string>>>,    // option, not a bare list
  payload: option<list<u8>>
}
```

So the snippet fails to compile three ways: `expected Verb, found String`,
`expected Option<Vec<(String, String)>>, found Vec<...>`, and `?` on a
`HttpError` that has no `From` impl.

**Repro:** paste the snippet into any contract that imports
`host:interfaces/http-with-placeholders@2.1.0` and run
`cargo build --target wasm32-wasip2 --release`.

**Working form** (what this contract uses, in `src/host_shim.rs`):

```rust
hwp::call(&hwp::Request {
    method: hwp::Verb::Post,
    url: url.to_string(),
    headers: Some(headers),
    payload: Some(payload),
}).map_err(format_http_error)?
```

The [write-contract](https://docs.terminal3.io/developers/adk/get-started/walkthrough/write-contract)
page gets this right, so the two pages disagree with each other.

---

## 3. `egress-denied`'s WIT doc comment contradicts the docs site

**Severity: low, but it points at the single most confusing part of the model.**

`host-interfaces-2.1.0/package.wit` documents the error as:

```wit
/// Target host is not on the contract's `http_allow_list`. Payload
/// is the offending host string for operator diagnostics.
egress-denied(string),
```

But [Outbound HTTP calls are authorized by the user, not the
contract](https://docs.terminal3.io/developers/adk/tips/outbound-http-auth-by-user)
states the opposite — that there is no contract-side allow list, and egress is
resolved per call from the caller's grant. The observed behaviour matches the
docs page: this contract declares no hosts anywhere, and its outbound call
succeeded only after the data owner's `agent-auth-update` named
`allowedHosts: ["httpbin.org"]`.

**Ask:** fix the WIT comment. It is the text a developer reads at the moment
the call fails, and it sends them looking for a manifest that does not exist.

---

## 4. Registration is the only way to learn a tail's `contract_id`, and map ACLs depend on it

**Severity: medium (a redeploy can silently break a working contract).**

The [register](https://docs.terminal3.io/developers/adk/get-started/walkthrough/register-contract)
page warns that re-registering a tail allocates a **new** `contract_id` and that
there is no API to read a tail's current id back. But
[Create Tenant KV Maps](https://docs.terminal3.io/developers/adk/tips/create-kv-maps)
says `MapAlreadyExists` is "idempotent — safe to re-run when re-deploying",
which reads as *nothing to do on redeploy*. Both cannot be comfortable at once:
the map does survive, but its `readers`/`writers` still name the previous
`contract_id`, and the KV governor defaults to deny.

**Mitigation used here:** `ops/deploy.ts` catches `MapAlreadyExists` and calls
`tenant.maps.update` with the freshly-returned `contract_id` on every deploy,
so the ACL can never lag the contract. That is a workaround for a missing
read API, not a fix.

**Ask:** a `contracts.get({ tail })` returning the current `contract_id`, or
map ACLs that can be scoped to a contract *tail* rather than a numeric id.

---

## 5. SDK errors arrive as a ~2 MB minified stack trace

**Severity: low (developer experience).**

An uncaught `RpcError` from `@terminal3/t3n-sdk@5.2.0` prints the offending line
of `dist/index.esm.js` — which is one minified line — so the terminal fills with
whitespace and a lone `^`. The captured output of a single failed demo run was
**2,425,210 bytes**, of which the useful part was the last 20 lines.

**Repro:** trip bug 1 (or any RPC error) without a `try/catch`.

**Ask:** ship a source map, or truncate the code frame. The error object itself
is good — `code`, `rpcMethod`, `detail`, `requestId` are all there and the
`requestId` is genuinely useful for support.

---

## 6. Docs gap: no worked example of the self-call (single-key) path

**Severity: low, but it is the first thing every solo developer needs.**

[Invoke your contract](https://docs.terminal3.io/developers/adk/get-started/walkthrough/invoke-contract)
requires three separate credentials — tenant, agent, user — and notes in one
sentence that a self-call sets `agentDid` to the user's own DID. Since an agent
key must be claimed separately and starts with a zero credit balance, the
single-key path is what a developer actually runs first, and there is no
end-to-end example of it.

`ops/demo.ts` in this repo is that example: one `T3N_API_KEY` acts as tenant,
data owner and caller, the grant is a self-grant, and the placeholder still
resolves — confirming the profile substitution works on the self-call path.

**Ask:** promote the self-call to a worked snippet on that page.

---

## Things that worked exactly as documented

Worth recording, because a bug log with no baseline is not useful:

- `wasm32-wasip2` + `crate-type = ["cdylib", "lib"]` emitted a real component
  (`0061 736d 0d00 0100`) with no `cargo-component` — 237,239 bytes in 25.8s.
- `tenant.contracts.register` → `contract_id 778` first try.
- `map-entry-set` seeded a contract-only map from the control plane, as
  documented, bypassing the `writers` ACL.
- `{{profile.verified_contacts.email.value}}` resolved inside the enclave: the
  provider echoed a real address that this contract's WASM never held.
- `kv-store::scan` returned the audit rows in sequence order on the first try.
