// End-to-end demo against the live cluster. Runs the whole story in order:
// grant -> record -> check -> send -> withdraw -> send (refused) -> audit.
//
//   T3N_API_KEY=... npx tsx ops/demo.ts
//
// This is a SELF call: the same key stands in for tenant, data owner and
// caller, so `agentDid` in the grant is our own DID. Split the identities
// (a separate agent key from the claim page) and nothing else changes.
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connectTenant, scriptName } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const PROVIDER_HOST = process.env.PROVIDER_HOST ?? "httpbin.org";
const SUBJECT = process.env.SUBJECT_REF ?? "cust-1042";
const CATEGORY = "billing";

const { t3n, tenant, tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
console.log(`contract ${SCRIPT} @ ${version}\n`);

const step = (n: string) => console.log(`\n--- ${n} ---`);
const call = (fn: string, input: unknown) =>
  t3n.executeAndDecode({ contract_id: SCRIPT, contract_version: version, function_name: fn, input });

step("0. the data owner authorises this caller for these functions and this host");
const userContractVersion = await getContractVersion(getNodeUrl(), "tee:user/contracts");
await t3n.execute({
  contract_id: "tee:user/contracts",
  contract_version: userContractVersion,
  function_name: "agent-auth-update",
  input: {
    agents: [
      {
        agentDid: tenantDid, // self-grant; a real agent uses its own DID here
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
});
console.log(`granted: ${SCRIPT} may reach ${PROVIDER_HOST}`);

step("1. record consent");
console.log(JSON.stringify(await call("record-consent", {
  subject_ref: SUBJECT,
  category: CATEGORY,
  granted: true,
  evidence_ref: "signup-form-v3",
}), null, 1));

step("2. check consent (read-only, no send)");
console.log(JSON.stringify(await call("check-consent", { subject_ref: SUBJECT, category: CATEGORY }), null, 1));

step("3. send a notice — allowed. The recipient is a {{profile.*}} marker.");
try {
  console.log(JSON.stringify(await call("send-notice", {
    subject_ref: SUBJECT,
    category: CATEGORY,
    template_id: "invoice_due",
    params: { invoice_no: "INV-2026-0042", amount_due: "129.00", currency: "GBP" },
  }), null, 1));
} catch (e: any) {
  console.log("send-notice returned an error:", String(e?.message ?? e));
}

step("4. a caller tries to smuggle the recipient in — refused before any host call");
try {
  await call("send-notice", {
    subject_ref: SUBJECT, category: CATEGORY, template_id: "invoice_due",
    params: { note: "reply to jane.doe@example.com" },
  });
  console.log("UNEXPECTED: the screen let it through");
} catch (e: any) {
  console.log("refused:", String(e?.message ?? e));
}

step("5. the customer withdraws consent");
console.log(JSON.stringify(await call("withdraw-consent", {
  subject_ref: SUBJECT, category: CATEGORY, evidence_ref: "unsubscribe-link",
}), null, 1));

step("6. the same send is now refused — the gate fails closed, nothing leaves");
try {
  console.log(JSON.stringify(await call("send-notice", {
    subject_ref: SUBJECT, category: CATEGORY, template_id: "invoice_due",
    params: { invoice_no: "INV-2026-0043" },
  }), null, 1));
} catch (e: any) {
  console.log("send-notice returned an error:", String(e?.message ?? e));
}

step("7. the audit ledger an auditor reads back");
console.log(JSON.stringify(await call("audit-log", { subject_ref: SUBJECT, limit: 20 }), null, 1));
