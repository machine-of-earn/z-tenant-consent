// Deliberate failure probes. Each one asserts a documented behaviour and
// prints exactly what the cluster said, so the bug log quotes reality.
import { getContractVersion, getNodeUrl } from "@terminal3/t3n-sdk";
import { connectTenant, scriptName } from "./session.js";

const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const { t3n, tenantDid } = await connectTenant();
const SCRIPT = scriptName(tenantDid, TAIL);
const version = await getContractVersion(getNodeUrl(), SCRIPT);
const call = (fn: string, input: unknown) =>
  t3n.executeAndDecode({ contract_id: SCRIPT, contract_version: version, function_name: fn, input });

async function probe(name: string, fn: () => Promise<unknown>) {
  process.stdout.write(`\n[probe] ${name}\n`);
  try {
    console.log("  OK:", JSON.stringify(await fn()).slice(0, 400));
  } catch (e: any) {
    console.log("  ERR:", String(e?.message ?? e).slice(0, 400));
  }
}

await probe("audit-log with an empty input object", () => call("audit-log", {}));
await probe("audit-log with no subject filter (whole ledger)", () => call("audit-log", { limit: 5 }));
await probe("a function name that does not exist", () => call("no-such-function", {}));
await probe("send-notice for a subject with no consent row at all", () =>
  call("send-notice", { subject_ref: "cust-never-seen", category: "billing", template_id: "invoice_due" }));
await probe("check-consent for a category never recorded", () =>
  call("check-consent", { subject_ref: "cust-1042", category: "marketing" }));
await probe("send-notice with a category that has no grant scope", () =>
  call("send-notice", { subject_ref: "cust-1042", category: "marketing", template_id: "x" }));
await probe("a 129-character subject_ref (over the contract's own cap)", () =>
  call("check-consent", { subject_ref: "x".repeat(129), category: "billing" }));
