// The real three-party flow: tenant / data owner / agent caller, three DISTINCT
// DIDs — the thing ops/demo.ts fakes with a self-call.
//
//   T3N_API_KEY=... T3N_DATAOWNER_KEY=... T3N_AGENT_KEY=... npx tsx ops/three-party.ts
//
// Identities (each is its own secp256k1 key and its own authenticated session):
//   tenant     — T3N_API_KEY, owns and deployed z:<tenant>:consent
//   data owner — T3N_DATAOWNER_KEY, the customer whose profile holds the contact
//   agent      — T3N_AGENT_KEY, the caller; never learns the contact
//
// Missing keys are generated locally and printed as export lines. A key made
// this way starts with ZERO credits (docs: /developers/agents/register-agent),
// so run it once, send the printed DIDs to Terminal 3 for a top-up, and run it
// again — the metered steps below only work on a funded DID.
//
// The run is written so every step prints what the cluster said, including the
// two that do NOT succeed. Steps 3-5 are the measurement that produced BUGS.md
// #9 and #10: outbound egress is resolved from the CALLER's own grant rather
// than the subject user's, and only the tenant's own session carries a user
// context that {{profile.*}} can resolve against. Step 6 is therefore the only
// identity that can complete a dispatch today, and step 7 reads the ledger the
// agent's consent row and the tenant's dispatch both landed in.
import { randomBytes } from "node:crypto";
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect, connectTenant, scriptName } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const PROVIDER_HOST = process.env.PROVIDER_HOST ?? "httpbin.org";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-3001";
const CATEGORY = "billing";
const FUNCTIONS = ["record-consent", "withdraw-consent", "check-consent", "send-notice", "audit-log"];

const newKey = () => "0x" + randomBytes(32).toString("hex");
const keyFor = (name: string) => {
  const v = process.env[name];
  if (v) return { key: v, fresh: false };
  const key = newKey();
  console.log(`# ${name} was not set — generated one (zero credits):\nexport ${name}=${key}`);
  return { key, fresh: true };
};

const step = (n: string) => console.log(`\n--- ${n} ---`);
const attempt = async (what: string, fn: () => Promise<unknown>) => {
  try {
    const out = await fn();
    console.log(`OK   ${what}: ${JSON.stringify(out).slice(0, 500)}`);
    return { ok: true, out };
  } catch (e: any) {
    const msg = String(e?.message ?? e);
    console.log(`FAIL ${what}: ${msg}`);
    return { ok: false, err: msg };
  }
};

step("0. three identities authenticate — free, no credits needed");
const { t3n: tenantT3n, tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
console.log(`tenant     ${tenantDid}`);
console.log(`contract   ${SCRIPT} @ ${version}`);

const owner = keyFor("T3N_DATAOWNER_KEY");
const agent = keyFor("T3N_AGENT_KEY");
const ownerSession = await connect(owner.key);
const agentSession = await connect(agent.key);
console.log(`data owner ${ownerSession.did}${owner.fresh ? "  (fresh key, 0 credits)" : ""}`);
console.log(`agent      ${agentSession.did}${agent.fresh ? "  (fresh key, 0 credits)" : ""}`);
if (new Set([tenantDid, ownerSession.did, agentSession.did]).size !== 3) {
  throw new Error("identities collided — this is supposed to be three distinct DIDs");
}

const userContractVersion = await getContractVersion(getNodeUrl(), "tee:user/contracts");
const grantFrom = (who: string, session: any, agentDid: string) =>
  attempt(`agent-auth-update signed by the ${who}`, () =>
    session.client.execute({
      contract_id: "tee:user/contracts",
      contract_version: userContractVersion,
      function_name: "agent-auth-update",
      input: {
        agents: [{ agentDid, scripts: [{ scriptName: SCRIPT, versionReq: version,
          functions: FUNCTIONS, allowedHosts: [PROVIDER_HOST] }] }],
      },
    }));

const noticeInput = (invoice: string) => ({
  subject_ref: SUBJECT, category: CATEGORY, template_id: "invoice_due",
  params: { invoice_no: invoice, amount_due: "42.00", currency: "GBP" },
});
const sendAs = (who: string, session: any, invoice: string) =>
  attempt(`send-notice called by the ${who}`, () =>
    (session.client ?? session).executeAndDecode({
      contract_id: SCRIPT, contract_version: version, function_name: "send-notice",
      input: noticeInput(invoice),
    }));

step("1. the data owner authorises the AGENT's DID (not its own) on our contract");
const grant = await grantFrom("data owner", ownerSession, agentSession.did);

step("2. the agent records the data owner's consent — called with the AGENT's session");
const rec = await attempt("record-consent", () =>
  agentSession.client.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "record-consent",
    input: { subject_ref: SUBJECT, category: CATEGORY, granted: true, evidence_ref: "signup-form-v3" },
  }));

// Determinism: a self-grant written by an earlier run would silently change what
// step 3 measures, so the agent clears its own policy document first. An empty
// `agents` list IS the revocation — the document is the whole state.
step("2b. the agent clears its own grant document, so ONLY the data owner's grant is in play");
const revoke = await attempt("agent-auth-update with an empty agents list, signed by the agent", () =>
  agentSession.client.execute({
    contract_id: "tee:user/contracts", contract_version: userContractVersion,
    function_name: "agent-auth-update", input: { agents: [] },
  }));

step("3. the agent sends the notice on the data owner's grant alone — egress is denied");
const send3 = await sendAs("agent, on the data owner's grant", agentSession, "INV-2026-0101");

step("4. the agent grants ITSELF the same hosts and retries — egress passes, user context does not");
const selfGrant = await grantFrom("agent (self-grant)", agentSession, agentSession.did);
const send4 = await sendAs("agent, on its own self-grant", agentSession, "INV-2026-0102");

step("5. the data owner calls it directly, self-granted — same wall");
const ownerSelfGrant = await grantFrom("data owner (self-grant)", ownerSession, ownerSession.did);
const send5 = await sendAs("data owner, self-granted", ownerSession, "INV-2026-0103");

step("6. the tenant dispatches — the one identity whose profile the enclave can resolve");
const send6 = await attempt("send-notice called by the tenant", () =>
  tenantT3n.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "send-notice",
    input: noticeInput("INV-2026-0104"),
  }));

step("7. the tenant reads the audit ledger back — the agent's consent row governed the dispatch");
const audit = await attempt("audit-log (tenant session)", () =>
  tenantT3n.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "audit-log",
    input: { subject_ref: SUBJECT, limit: 20 },
  }));

step("result");
console.log(JSON.stringify({
  tenantDid, dataOwnerDid: ownerSession.did, agentDid: agentSession.did,
  authenticated_without_credits: true,
  data_owner_grants_agent: grant.ok,
  agent_revokes_own_grant: revoke.ok,
  agent_records_consent: rec.ok,
  agent_send_on_owner_grant: send3.ok,
  agent_self_grant: selfGrant.ok,
  agent_send_on_self_grant: send4.ok,
  owner_self_grant: ownerSelfGrant.ok,
  owner_send_self_granted: send5.ok,
  tenant_send: send6.ok,
  audit_log: audit.ok,
  errors: [send3, send4, send5].filter((r: any) => !r.ok).map((r: any) => r.err),
}, null, 1));
