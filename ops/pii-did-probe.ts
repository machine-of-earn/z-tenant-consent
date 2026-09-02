// Can a caller name the subject user? The SDK's relay wire (InvokeRequest) has
// `pii_did` — "Delegated-call target member; omitted for a self (non-delegated)
// call" — but the session execute path is undocumented on this point. Try it.
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect, connectTenant, scriptName, requireEnv } from "./session.js";
const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-3001";
const { tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
const owner = await connect(requireEnv("T3N_DATAOWNER_KEY"));
const agent = await connect(requireEnv("T3N_AGENT_KEY"));
console.log(`contract ${SCRIPT} @ ${version}\nowner ${owner.did}\nagent ${agent.did}`);
const attempt = async (what: string, fn: () => Promise<unknown>) => {
  try { const o = await fn(); console.log(`OK   ${what}: ${JSON.stringify(o).slice(0,300)}`); }
  catch (e: any) { console.log(`FAIL ${what}: ${String(e?.message ?? e).slice(0,300)}`); }
};
const notice = { subject_ref: SUBJECT, category: "billing", template_id: "invoice_due",
  params: { invoice_no: "INV-2026-0105", amount_due: "42.00", currency: "GBP" } };
for (const [label, extra] of [
  ["pii_did = the data owner", { pii_did: owner.did }],
  ["pii_did = the tenant", { pii_did: tenantDid }],
  ["user_did = the data owner", { user_did: owner.did }],
  ["on_behalf_of = the data owner", { on_behalf_of: owner.did }],
] as [string, Record<string, string>][]) {
  await attempt(`agent send-notice with ${label}`, () => agent.client.executeAndDecode({
    contract_id: SCRIPT, contract_version: version, function_name: "send-notice",
    ...extra, input: notice }));
}
await attempt("delegation.check — is the agent authorised to act for the data owner?", () =>
  (agent.client as any).checkDelegation?.({ contract: SCRIPT, pii_did: owner.did,
    functions: ["send-notice"], scopes: [] }) ?? Promise.reject(new Error("checkDelegation not on the client")));
