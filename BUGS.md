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

## 7. A zero-credit DID *can* sign `agent-auth-update` — the docs say it cannot

The docs are emphatic that an identity created outside the claim page is unusable:
*"A key generated any other way ... starts with **zero** credits and can't pay for
this step"* (`/developers/agents/register-agent`), and the common-errors table
attributes `InsufficientCreditError` on agent calls to exactly that.

Measured on testnet 2026-09-01 with two keys generated locally
(`randomBytes(32)`), neither ever touched by the claim page:

| Step | Identity | Result |
|---|---|---|
| handshake + `authenticate` | data owner, 0 credits | OK — DID `did:t3n:be3b5c5d…` issued |
| handshake + `authenticate` | agent, 0 credits | OK — DID `did:t3n:287c54c6…` issued |
| `tee:user/contracts` → `agent-auth-update` | data owner, 0 credits | **OK — `tx:121:176140`** |
| `record-consent` on our contract | agent, 0 credits | `InsufficientCredit (required=10000000000, available=0)` |
| `send-notice` on our contract | agent, 0 credits | `InsufficientCredit (required=10000000000, available=0)` |

So the credit gate is on *contract invocation*, not on writes in general: the data
owner's authorisation grant — a state-changing write to a system contract — went
through on an account with a zero balance. A developer reading the current text
would expect step 3 to fail too, and would go get a claim-page key before
discovering that the free part of the three-party flow is already reachable.

Also useful, and not stated anywhere we found: the metered price of one contract
call is exactly **10,000,000,000 base units**, and the error names the account,
the requirement and the balance — a good error, worth keeping.

**Ask:** say "invoking a contract is metered; identity creation and
`agent-auth-update` are not", and state the per-call price.

---

## 8. The prerequisites page never names a minimum Node version, and the SDK hard-fails on Node 18

`@terminal3/t3n-sdk@5.2.0` on Node **18.19.1** (Debian's current default) throws
before any network call:

```
Error: Crypto API is not available in Node.js 18.19.1. The crypto module should be
available. If you're using Node.js 16-18, ensure you're importing this SDK correctly.
```

The message reads as an import mistake, but the import is the documented one, and
the same file runs unchanged on Node 22.14.0. Node 18 only exposes
`globalThis.crypto` behind `--experimental-global-webcrypto`, so this is a version
floor, not a usage error. `Set Up Development Environment` lists no Node version
requirement at all, and `package.json` in the SDK declares no `engines` field.

**Ask:** put `"engines": { "node": ">=20" }` in the SDK package and one line on the
prerequisites page. Re-word the error to name the real cause.

---

## 9. Outbound egress is resolved from the CALLER's own grant, not the subject user's

**Severity: high for anyone building the documented agent flow — the delegated
call the docs describe cannot reach the network.**

[Outbound HTTP calls are authorized by the user, not the
contract](https://docs.terminal3.io/developers/adk/tips/outbound-http-auth-by-user)
states the rule in two lines:

> * **Delegated call** → the subject user's grant.
> * **Direct (self) call** → the caller's own self-grant.

Measured on testnet 2026-09-02, three distinct DIDs, one contract
(`z:9e0cfe5b…:consent @ 0.1.0`), one host (`httpbin.org`):

| # | What was in place | Caller | Result |
|---|---|---|---|
| 1 | data owner's grant names the agent, `allowedHosts:["httpbin.org"]` (`tx:121:176670`) | agent | `egress_denied: host 'httpbin.org' is not on the caller's allowed-hosts grant` |
| 2 | same, re-written with `versionReq: null` (match any version) (`tx:121:176583`) | agent | `egress_denied` — unchanged |
| 3 | same grant, retried with no new writes | agent | `egress_denied` — so it is not propagation delay |
| 4 | agent additionally grants **itself** the same host (`tx:121:176683`) | agent | egress passes; the call now fails one layer later (see #10) |

The stored document is not the problem — read back with `agent-auth-get` on the
data owner's session, it holds exactly what was written:

```json
{"agents":[{"agent_did":"did:t3n:287c54c6196551bd61bb5c48872be66610a95ddf",
  "scripts":[{"script_name":"z:9e0cfe5b257503525ad98c65e6ab7d7f09fbf620:consent",
  "version_req":"0.1.0","functions":["record-consent","withdraw-consent",
  "check-consent","send-notice","audit-log"],…}]}]}
```

The same session reads `null` from the agent's own document, so before step 4 the
agent held no self-grant at all — and that is the single variable that changed
the outcome. Note also that the agent's `record-consent` call **succeeded**
throughout on the data owner's grant, so the function half of the grant is honoured
across identities; only the egress half is read off the caller.

This inverts the security story on the page: the point of the model is that the
data owner decides which hosts an agent may reach on their behalf. As it behaves
today, the agent authorises its own egress and the data owner's list is not
consulted.

**Ask:** either make the delegated path resolve the subject user's grant as
documented, or change the page to say egress is always the caller's own — and
add the note to
[Agent Auth](https://docs.terminal3.io/developers/adk/overview/agent-auth-adk),
whose worked example is the delegated shape.

Reproduce: `ops/three-party.ts` steps 2b-4 (it revokes the agent's own document
first so the measurement is deterministic), or `ops/egress-probe.ts`, which rules
out propagation and `versionReq` separately. Request ids from the run:
`f658a52d-462d-4386-b0fc-b82945e80d91` (denied),
`5ca1c699-a8b3-4927-98a6-25213fb461c6` (past egress, next error).

---

## 10. Only the tenant's own session carries a user context, so `{{profile.*}}` cannot resolve for anyone else

**Severity: high — it is what stops a three-party flow from completing end to end.**

With egress open (finding #9, step 4), `send-notice` fails at placeholder
resolution for every identity except the tenant that owns the contract:

| Caller | Self-grant in place | Result |
|---|---|---|
| agent `did:t3n:287c54c6…` | yes (`tx:121:176683`) | `placeholder_no_user_context` — request `5ca1c699-…` |
| data owner `did:t3n:be3b5c5d…` | yes (`tx:121:176688`) | `placeholder_no_user_context` — request `924b58e1-…` |
| tenant `did:t3n:9e0cfe5b…` | yes | **dispatched**, provider HTTP 200, audit row `a/cust-3001/00000000000000176692` |

`placeholder_no_user_context` is this contract's rendering of the host's
`placeholder-no-user-context` variant: *no user is bound to the invocation, so
there is no profile to resolve from*. The data owner is a fully authenticated
user session calling its own contract function directly — the documented
"direct (self) call" — and it still has no user bound.

[Placeholders in outbound calls](https://docs.terminal3.io/developers/adk/tips/placeholders-outbound-calls)
says the marker resolves against the calling user's profile, and
[Agent Auth](https://docs.terminal3.io/developers/adk/overview/agent-auth-adk)
describes an agent acting for a data owner. Together they read as: an agent calls,
the data owner's contact is substituted inside the enclave, the agent never sees
it. That is exactly the shape this contract was written for, and it is the one
shape that cannot run today.

Two possibilities we cannot distinguish from outside, and either answer is a
docs fix: (a) user context is bound only for the tenant DID on its own contract,
or (b) a fresh DID with no profile record binds no context and the error is
reported as *no context* rather than *no such field* (the host has a separate
`placeholder-unknown(field)` variant for that, which we never saw).

**Settled the same day, and it is not an authorisation problem.** The execute
wire does have a subject argument — `pii_did`, "delegated-call target member;
omitted for a self (non-delegated) call" (`InvokeRequest` in the SDK types; it is
not on any docs page we could find). Naming the subject and holding a grant the
node itself calls valid still does not bind a user:

| Delegation state | `delegation.check` | agent's `send-notice` with `pii_did` |
|---|---|---|
| `agent-auth-update` grant only (the documented surface) | `authorised:false, disclosed:false` | `Forbidden (agent_auth_not_found)` — see #11 |
| `member-delegation-update` grant (the current surface) | `authorised:true, disclosed:true, satisfied:[member_delegation]` | `placeholder_no_user_context` |

So the authorisation edge can be made complete and correct, the call is accepted
as a delegated call, and the enclave still has no profile to resolve against.

**Ask:** state which identity's profile a contract's placeholders resolve
against on a delegated call, and whether binding a subject user is supported in
this build at all. If it is not yet, say so on the Agent Auth page — its worked
example is exactly this shape. Document `pii_did` either way.

Reproduce: `ops/three-party.ts` steps 4-7 and `ops/delegated-send.ts` (which
writes the grant on the current surface and calls with the subject named); full
transcript in `docs/three-party-transcript.txt`.

---

## 11. The delegated call reads a different grant store than the one the docs teach

**Severity: high — every worked example on the docs site writes to the surface the
delegated path does not read.**

The docs teach `agent-auth-update` on `tee:user/contracts`
([Agent Auth](https://docs.terminal3.io/developers/adk/overview/agent-auth-adk),
[Invoke your contract](https://docs.terminal3.io/developers/adk/get-started/walkthrough/invoke-contract)).
The SDK marks that whole module **deprecated**, in favour of
`member-delegation-get` / `member-delegation-update` on
`tee:authorisations/contracts` (`T3nClient.updateMemberDelegation`).

Measured 2026-09-02, same agent, same subject, same contract, same functions:

| Grant written by the data owner | `delegation.check` verdict | agent's delegated call |
|---|---|---|
| `agent-auth-update` (documented), read back intact with `agent-auth-get` | `authorised:false, disclosed:false, satisfied:[], missing:[]` | `Forbidden (agent_auth_not_found): did:t3n:287c54c6… not permitted to act on behalf of did:t3n:be3b5c5d… for z:…:consent send-notice` |
| `member-delegation-update` (current), same fields | `authorised:true, disclosed:true, satisfied:[{grant:"member_delegation",…}]` | passes the authorisation gate |

The first row is the state a developer reaches by following the documentation
exactly. The node's own answer to "is this agent allowed to act for this member"
is **no**, while the grant sits readable in the store the docs told them to write.
Two failure modes follow from that, and we hit both: a *self*-call path where the
`agent-auth-update` grant IS honoured for functions and egress (findings #9, #10),
and a *delegated* path where it is not seen at all — so the same document behaves
differently depending on how the call is made.

Note also that `updateMemberDelegation` attaches a validity window on its own
(`valid_from_secs 1788315711`, `valid_until_secs 1796091711` — 90 days) where the
`agent-auth-update` write we made carried none. That default is not stated
anywhere we read.

**Ask:** point the walkthroughs at the surface the delegated path actually reads,
or make `agent-auth-update` write through to it during the deprecation window.
Whichever way, `delegation.check` is the fastest debugging tool on the platform
and deserves a docs page — it answered in one call what four `send-notice`
attempts could only hint at.

Reproduce: `ops/delegated-send.ts`.

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
