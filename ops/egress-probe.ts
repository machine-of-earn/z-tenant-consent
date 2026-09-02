// Why does the agent's send-notice hit egress_denied when the data owner's
// grant names httpbin.org?
//
//   T3N_API_KEY=... T3N_DATAOWNER_KEY=... T3N_AGENT_KEY=... npx tsx ops/egress-probe.ts
//
// Three candidate causes, tested in order, cheapest first:
//   T1  the grant had not propagated when the call went out (retry, no writes)
//   T2  versionReq is matched strictly (re-grant with versionReq null = any)
//   T3  a delegated call is resolved against the CALLER's own self-grant, so
//       the agent must also grant itself (agent signs its own agent-auth-update)
// Between each, read the stored policy back with agent-auth-get so the run
// records what the cluster actually holds, not what we think we wrote.
//
// Precondition: the agent must hold NO self-grant when this starts, or T1
// measures the wrong thing — ops/three-party.ts clears it (step 2b) before its
// own version of this measurement. Run that script for the deterministic form;
// this one exists to rule out propagation and versionReq one at a time.
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect, connectTenant, scriptName, requireEnv } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const PROVIDER_HOST = process.env.PROVIDER_HOST ?? "httpbin.org";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-3001";
const CATEGORY = "billing";
const FUNCTIONS = ["record-consent", "withdraw-consent", "check-consent", "send-notice", "audit-log"];

const { tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
const ownerSession = await connect(requireEnv("T3N_DATAOWNER_KEY"));
const agentSession = await connect(requireEnv("T3N_AGENT_KEY"));
const userContractVersion = await getContractVersion(getNodeUrl(), "tee:user/contracts");
console.log(`contract   ${SCRIPT} @ ${version}`);
console.log(`data owner ${ownerSession.did}`);
console.log(`agent      ${agentSession.did}`);

const attempt = async (what: string, fn: () => Promise<unknown>) => {
  try {
    const out = await fn();
    console.log(`OK   ${what}: ${JSON.stringify(out).slice(0, 300)}`);
    return { ok: true, out: JSON.stringify(out).slice(0, 300) };
  } catch (e: any) {
    const msg = String(e?.message ?? e).slice(0, 300);
    console.log(`FAIL ${what}: ${msg}`);
    return { ok: false, err: msg };
  }
};

const sendNotice = (tag: string) =>
  attempt(`send-notice as the agent (${tag})`, () =>
    agentSession.client.executeAndDecode({
      contract_id: SCRIPT, contract_version: version, function_name: "send-notice",
      input: { subject_ref: SUBJECT, category: CATEGORY, template_id: "invoice_due",
               params: { invoice_no: "INV-2026-0091", amount_due: "42.00", currency: "GBP" } },
    }));

const readPolicy = (who: string, s: any, contract: string) =>
  attempt(`agent-auth-get on ${contract} as the ${who}`, () =>
    s.client.executeAndDecode({
      contract_id: contract, contract_version: userContractVersion,
      function_name: "agent-auth-get", input: {},
    }));

const grantFrom = (who: string, s: any, agentDid: string, versionReq: string | null) =>
  attempt(`agent-auth-update signed by the ${who} (agentDid=${agentDid.slice(0, 16)}…, versionReq=${JSON.stringify(versionReq)})`, () =>
    s.client.execute({
      contract_id: "tee:user/contracts", contract_version: userContractVersion,
      function_name: "agent-auth-update",
      input: { agents: [{ agentDid, scripts: [{ scriptName: SCRIPT, versionReq,
        functions: FUNCTIONS, allowedHosts: [PROVIDER_HOST] }] }] },
    }));

const results: Record<string, unknown> = {};

console.log("\n--- what the cluster actually holds right now ---");
results.owner_policy_user = await readPolicy("data owner", ownerSession, "tee:user/contracts");
results.owner_policy_auth = await readPolicy("data owner", ownerSession, "tee:authorisations/contracts");
results.agent_policy_user = await readPolicy("agent", agentSession, "tee:user/contracts");

console.log("\n--- T1: retry with the grant already in place (propagation?) ---");
results.t1 = await sendNotice("T1 retry, no new writes");

if (!(results.t1 as any).ok) {
  console.log("\n--- T2: re-grant from the data owner with versionReq null (match any version) ---");
  results.t2_grant = await grantFrom("data owner", ownerSession, agentSession.did, null);
  results.t2 = await sendNotice("T2 after versionReq null");
}

if (!((results.t2 as any)?.ok)) {
  console.log("\n--- T3: the agent grants ITSELF (caller self-grant) ---");
  results.t3_grant = await grantFrom("agent", agentSession, agentSession.did, null);
  results.t3 = await sendNotice("T3 after the agent's own self-grant");
}

console.log("\n--- result ---");
console.log(JSON.stringify({
  contract: SCRIPT, version,
  dataOwnerDid: ownerSession.did, agentDid: agentSession.did,
  t1_retry: (results.t1 as any)?.ok ?? null,
  t2_version_req_null: (results.t2 as any)?.ok ?? null,
  t3_agent_self_grant: (results.t3 as any)?.ok ?? null,
}, null, 1));
