// The delegated send, on the CURRENT delegation surface.
//
// `agent-auth-update` (what the docs walk you through) is the deprecated
// camelCase surface. The SDK's live one is `member-delegation-update` —
// T3nClient.updateMemberDelegation(BoundGrant) — and the execute wire carries
// `pii_did`, "delegated-call target member; omitted for a self call".
// This script writes the grant on that surface and makes the call with the
// subject named, which is the shape the docs describe but never spell out.
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect, connectTenant, scriptName, requireEnv } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const PROVIDER_HOST = process.env.PROVIDER_HOST ?? "httpbin.org";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-3001";
const FUNCTIONS = ["record-consent", "withdraw-consent", "check-consent", "send-notice", "audit-log"];

const { t3n: tenantT3n, tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
const owner = await connect(requireEnv("T3N_DATAOWNER_KEY"));
const agent = await connect(requireEnv("T3N_AGENT_KEY"));
console.log(`contract   ${SCRIPT} @ ${version}`);
console.log(`tenant     ${tenantDid}\ndata owner ${owner.did}\nagent      ${agent.did}`);

const attempt = async (what: string, fn: () => Promise<unknown>) => {
  try { const o = await fn(); console.log(`OK   ${what}: ${JSON.stringify(o).slice(0, 400)}`); return true; }
  catch (e: any) { console.log(`FAIL ${what}: ${String(e?.message ?? e).slice(0, 400)}`); return false; }
};
const send = (label: string, session: any, pii?: string) =>
  attempt(`send-notice ${label}`, () => session.client.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "send-notice",
    ...(pii ? { pii_did: pii } : {}),
    input: { subject_ref: SUBJECT, category: "billing", template_id: "invoice_due",
             params: { invoice_no: "INV-2026-0106", amount_due: "42.00", currency: "GBP" } },
  }));

console.log("\n--- 1. the data owner writes the grant on the member-delegation surface ---");
await attempt("updateMemberDelegation (data owner → agent)", () =>
  (owner.client as any).updateMemberDelegation({
    grantee: agent.did, contract_id: SCRIPT, functions: FUNCTIONS, scopes: [],
    allowed_hosts: [PROVIDER_HOST],
  }, { discoverDids: [owner.did] }));

console.log("\n--- 2. read it back ---");
await attempt("getMemberDelegation (data owner)", () => (owner.client as any).getMemberDelegation());

console.log("\n--- 3. is the agent authorised to act for the data owner now? ---");
await attempt("delegation.check", () => (agent.client as any).checkDelegation({
  contract: SCRIPT, pii_did: owner.did, functions: ["send-notice"], scopes: [] }));

console.log("\n--- 4. the agent sends WITH the subject named ---");
await send("as the agent, pii_did = the data owner", agent, owner.did);

console.log("\n--- 5. the same call without the subject, for contrast ---");
await send("as the agent, no pii_did", agent);

console.log("\n--- 6. the audit ledger ---");
await attempt("audit-log (tenant)", () => tenantT3n.executeAndDecode({
  contract_id: SCRIPT, contract_version: version, function_name: "audit-log",
  input: { subject_ref: SUBJECT, limit: 5 } }));
