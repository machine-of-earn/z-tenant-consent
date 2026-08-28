# z-tenant-consent — an enterprise agent that cannot send a notice without consent, and never sees who it is sending to

**Repo (public):** https://github.com/machine-of-earn/z-tenant-consent
**Live on testnet:** `z:9e0cfe5b257503525ad98c65e6ab7d7f09fbf620:consent`, contract id **778**
**Bug log:** https://github.com/machine-of-earn/z-tenant-consent/blob/main/BUGS.md
**Built:** 2026-08-28, `@terminal3/t3n-sdk@5.2.0`, `wit-bindgen 0.49`, `wasm32-wasip2`.

## The problem we picked, and why

Enterprises want AI agents to talk to their customers. The blocker is not
capability, it is that three requirements normally cannot hold at once:

1. the agent must never learn the customer's contact details — once an address
   is in an agent's context it is in a log, a prompt cache and a model
   provider's infrastructure;
2. every send must be provably gated on a recorded, unexpired consent for that
   exact notice category, and must fail closed;
3. an auditor must be able to read back what was consented and what was sent,
   without trusting the operator's own database.

On ordinary infrastructure you get these by discipline, and discipline is what
fails. On T3N they come from the platform. That is why we did not build the
payroll agent the docs already ship: the interesting thing T3N can do that
nothing else can is `http-with-placeholders`, and this use case is the shortest
honest path to showing it.

## What it is

A TEE contract with five functions — `record-consent`, `withdraw-consent`,
`check-consent`, `send-notice`, `audit-log` — plus a one-command deploy and a
seven-step live demo script.

| Requirement | How it is enforced — not by review, by the platform |
|---|---|
| The agent never sees the recipient | The contract templates `{{profile.verified_contacts.email.value}}` into the provider request. The host resolves it inside the enclave. |
| No code path *can* send a plaintext recipient | The plain `http` interface is **not imported**. Capabilities come from WIT imports, so the host refuses to load a contract that uses one it did not declare. There is no code path to review. |
| A caller cannot smuggle a recipient in | Every caller-supplied parameter key *and value* is screened; nested objects are refused rather than walked. An address hidden in prose ("reply to jane@example.com") is caught. |
| The gate fails closed | Absent, withdrawn and expired consent are all refusals, and the refusal returns **before** any request is built — a refused notice does not even tell the provider the customer exists. |
| Audit survives the operator | Every decision, sent and refused, appends a row keyed by the cluster's own sequence number. |

## It runs. Here is the run.

```
--- 3. send a notice — allowed. The recipient is a {{profile.*}} marker. ---
{ "dispatched": true, "provider_status": 200, "reason": "consent_valid",
  "audit_key": "a/cust-1042/00000000000000156661" }

--- 4. a caller tries to smuggle the recipient in — refused before any host call ---
refused: contract error: bad input: params value under 'note' looks like an
email address; the recipient is resolved host-side from the user profile

--- 6. the same send is now refused — the gate fails closed, nothing leaves ---
{ "dispatched": false, "reason": "consent_withdrawn" }

--- 7. the audit ledger an auditor reads back ---
{ "count": 2, "rows": [ { "outcome": "dispatched", "provider_status": 200, ... },
                        { "outcome": "refused", "reason": "consent_withdrawn", ... } ] }
```

**The single most important line in the whole run** is in the provider's echo of
step 3: `"to": "machinamachinery@gmail.com"`. That is a real address. Grep the
contract source for it and it is not there — what the WASM held was the marker.
The host substituted the value inside the enclave, on the self-call path, with
the data owner's `agent-auth-update` grant naming both the functions and the
one host the contract may reach.

Full transcript: `docs/demo-transcript.txt` in the repo.

## Build quality and maintenance — the part that matters after the challenge

- **24 unit tests, on the native target**, with no cluster, no network and no
  WASM runtime. Every rule that matters is a pure function in `src/model.rs`,
  and the host is reached through a single shim whose native stubs fail loudly.
  `cargo test` is a complete check of the guards.
- **No off-cluster state.** Two tenant KV maps. No database, no queue, no cron,
  nothing to back up.
- **No secrets in the artifact.** The WASM holds no key and no URL; both are
  read from a contract-only map at call time. Rotating a provider key or
  switching email providers is a control-plane write, not a deployment.
- **Redeploy is one command** — and `ops/deploy.ts` re-points both map ACLs at
  the new `contract_id` on every run, which is the one redeploy hazard the
  platform currently has (bug 4 below).
- **Named failure modes.** `no_consent_on_record`, `consent_withdrawn`,
  `consent_expired`, `egress_denied`, `placeholder_unknown`,
  `placeholder_no_user_context` are stable strings that appear in both the
  response and the audit row.
- **Stated limits, not hidden ones.** `kv-store::scan` has no cursor, so
  `audit-log` caps at 500 rows and sets `"truncated": true` rather than
  pretending it saw everything.

## Bugs found

Six, each with a reproduction and the exact error text, in `BUGS.md`:

1. **Undocumented per-minute fuel quota.** Running the demo twice inside a
   minute fails with `quota exceeded (fuel_per_minute)`. `fuel_per_minute`
   appears nowhere in the docs, which describe credits as a depleting balance —
   so a developer cannot tell a rate limit from an exhausted account. **High.**
2. **The placeholders tip page shows Rust that does not compile.** It builds the
   request with `method: "POST".to_string()` and a bare `headers: vec![…]`; the
   vendored WIT declares `method: verb` and `headers: option<…>`. Three compile
   errors from a copy-paste. The write-contract page gets it right, so the two
   pages disagree.
3. **`egress-denied`'s WIT doc comment contradicts the docs site** — it says the
   host was "not on the contract's `http_allow_list`", while the documentation
   correctly says egress comes from the caller's grant and no contract-side list
   exists. It is the text a developer reads at the moment the call fails.
4. **A tail's `contract_id` can only be learned by registering it**, and map
   ACLs are scoped to that id — so a redeploy can leave a working contract
   unable to read its own maps, while `create-kv-maps` describes
   `MapAlreadyExists` as "safe to re-run when re-deploying".
5. **SDK errors arrive as a ~2 MB minified stack trace** — one failed run
   produced 2,425,210 bytes of output, of which 20 lines mattered.
6. **No worked example of the self-call (single-key) path**, which is what every
   solo developer runs first, since an agent key must be claimed separately and
   starts with zero credits. `ops/demo.ts` is that example.

Things that worked exactly as documented are listed too — a bug log with no
baseline is not useful.

## Handover

Keep running or hand over: both are one command. See the answer to question 3
above.
