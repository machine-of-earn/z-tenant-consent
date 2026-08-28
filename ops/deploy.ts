// One-command deploy: register the contract, create its maps, seed the
// provider config. Idempotent — safe to re-run after a rebuild.
//
//   T3N_API_KEY=... PROVIDER_URL=... npx tsx ops/deploy.ts
import { readFile } from "fs/promises";
import { connectTenant, requireEnv, scriptName } from "./session.js";

const WASM_PATH =
  process.env.WASM_PATH ?? "target/wasm32-wasip2/release/z_tenant_consent.wasm";
const TAIL = process.env.CONTRACT_TAIL ?? "consent";
const VERSION = requireEnv("CONTRACT_VERSION");

const { tenant, tenantDid } = await connectTenant();
console.log("tenant:", tenantDid);

const wasm = await readFile(WASM_PATH);
console.log(`wasm: ${WASM_PATH} (${wasm.length} bytes)`);

const registered = await tenant.contracts.register({ tail: TAIL, version: VERSION, wasm });
const contractId = registered.contract_id;
console.log(`registered ${scriptName(tenantDid, TAIL)} v${VERSION} as contract id ${contractId}`);

// readers MUST be set explicitly — the KV governor defaults to deny.
for (const tail of ["ledger", "secrets"]) {
  try {
    await tenant.maps.create({
      tail,
      visibility: "private",
      writers: { only: [contractId] },
      readers: { only: [contractId] },
    });
    console.log(`map created: z:<tid>:${tail}`);
  } catch (e: any) {
    const msg = String(e?.message ?? e);
    if (!/MapAlreadyExists|already exists/i.test(msg)) throw e;
    // The map survives a re-register, but its ACL still points at the OLD
    // contract id, so the new build could not read it. Re-point it.
    await tenant.maps.update({
      tail,
      visibility: "private",
      writers: { only: [contractId] },
      readers: { only: [contractId] },
    });
    console.log(`map existed: z:<tid>:${tail} — ACL re-pointed at contract id ${contractId}`);
  }
}

// Control-plane writes bypass the writers ACL, which is how a contract-only
// map gets seeded from outside.
await tenant.executeControl("map-entry-set", {
  map_name: tenant.canonicalName("secrets"),
  key: "provider_url",
  value: requireEnv("PROVIDER_URL"),
});
console.log("seeded provider_url");

if (process.env.PROVIDER_API_KEY) {
  await tenant.executeControl("map-entry-set", {
    map_name: tenant.canonicalName("secrets"),
    key: "provider_api_key",
    value: process.env.PROVIDER_API_KEY,
  });
  if (process.env.PROVIDER_AUTH_HEADER) {
    await tenant.executeControl("map-entry-set", {
      map_name: tenant.canonicalName("secrets"),
      key: "provider_auth_header",
      value: process.env.PROVIDER_AUTH_HEADER,
    });
  }
  console.log("seeded provider_api_key (sealed in z:<tid>:secrets — not readable from outside)");
}

console.log(JSON.stringify({ contractId, tail: TAIL, version: VERSION, script: scriptName(tenantDid, TAIL) }));
