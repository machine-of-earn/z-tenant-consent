// The real three-party flow: tenant / data owner / agent caller, three DISTINCT
// DIDs — the thing ops/demo.ts fakes with a self-call.
//
//   T3N_API_KEY=... npx tsx ops/three-party.ts
//
// Identities (each is its own secp256k1 key and its own authenticated session):
//   tenant     — T3N_API_KEY, owns and deployed z:<tenant>:consent
//   data owner — T3N_DATAOWNER_KEY, the customer whose profile holds the contact
//   agent      — T3N_AGENT_KEY, the caller; never learns the contact
//
// Missing keys are generated locally and printed as export lines. A key made
// this way has ZERO credits (docs: /developers/agents/register-agent), so every
// metered step below is expected to fail with InsufficientCreditError until the
// DID is funded from the claim page. That failure is the point of the run: it
// records exactly which steps are free, which are metered, and what each one
// says, without pretending the flow is proven.
import { randomBytes } from "node:crypto";
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect, connectTenant, scriptName } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const PROVIDER_HOST = process.env.PROVIDER_HOST ?? "httpbin.org";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-3001";
const CATEGORY = "billing";

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
    console.log(`OK   ${what}: ${JSON.stringify(out)}`);
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

step("1. the data owner authorises the AGENT's DID (not its own) on our contract");
const grant = await attempt("agent-auth-update signed by the data owner", () =>
  ownerSession.client.execute({
    contract_id: "tee:user/contracts",
    contract_version: userContractVersion,
    function_name: "agent-auth-update",
    input: {
      agents: [
        {
          agentDid: agentSession.did,
          scripts: [
            {
              scriptName: SCRIPT,
              versionReq: version,
              functions: ["record-consent", "withdraw-consent", "check-consent", "send-notice", "audit-log"],
              allowedHosts: [PROVIDER_HOST],
            },
          ],
        },
      ],
    },
  }),
);

step("2. the agent records the data owner's consent — called with the AGENT's session");
const call = (fn: string, input: unknown) =>
  agentSession.client.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: fn, input,
  });
const rec = await attempt("record-consent", () =>
  call("record-consent", { subject_ref: SUBJECT, category: CATEGORY, granted: true, evidence_ref: "signup-form-v3" }));

step("3. the agent sends the notice — the recipient stays inside the enclave");
const send = await attempt("send-notice", () =>
  call("send-notice", {
    subject_ref: SUBJECT, category: CATEGORY, template_id: "invoice_due",
    params: { invoice_no: "INV-2026-0090", amount_due: "42.00", currency: "GBP" },
  }));

step("4. the tenant reads the audit ledger back — tenant session, has credits");
const audit = await attempt("audit-log (tenant session)", () =>
  tenantT3n.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "audit-log",
    input: { subject_ref: SUBJECT, limit: 20 },
  }));

step("result");
console.log(JSON.stringify({
  tenantDid, dataOwnerDid: ownerSession.did, agentDid: agentSession.did,
  authenticated_without_credits: true,
  grant: grant.ok, record_consent: rec.ok, send_notice: send.ok, audit_log: audit.ok,
  errors: [grant, rec, send, audit].filter((r: any) => !r.ok).map((r: any) => r.err),
}, null, 1));
