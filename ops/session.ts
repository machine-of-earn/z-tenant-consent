// Shared session bootstrap: one authenticated T3nClient + TenantClient.
//
// Every script in ops/ starts here so there is exactly one place that knows
// how to connect. The tenant DID is always read back from the authenticated
// session — never constructed, never hardcoded.
import {
  T3nClient,
  TenantClient,
  setEnvironment,
  loadWasmComponent,
  eth_get_address,
  metamask_sign,
  createEthAuthInput,
  fetchTrustedManifest,
  getNodeUrl,
} from "@terminal3/t3n-sdk";

export const ENV = (process.env.T3N_ENV ?? "testnet") as "testnet" | "production";

export async function connect(apiKey: string) {
  setEnvironment(ENV);
  const wasmComponent = await loadWasmComponent();
  const address = eth_get_address(apiKey);
  const client = new T3nClient({
    trustAnchor: await fetchTrustedManifest(ENV),
    wasmComponent,
    handlers: { EthSign: metamask_sign(address, undefined, apiKey) },
  });
  await client.handshake();
  const did = await client.authenticate(createEthAuthInput(address));
  return { client, wasmComponent, address, did: did.value as string };
}

export async function connectTenant() {
  const apiKey = requireEnv("T3N_API_KEY");
  const { client, wasmComponent, address, did } = await connect(apiKey);
  const tenant = new TenantClient({ t3n: client, baseUrl: getNodeUrl(), tenantDid: did });
  await tenant.tenant.me(); // throws if the DID is not an admitted tenant
  return { t3n: client, tenant, tenantDid: did, wasmComponent, address };
}

export function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set — export it before running this script`);
  return v;
}

export const tid = (did: string) => did.slice("did:t3n:".length);
export const scriptName = (did: string, tail: string) => `z:${tid(did)}:${tail}`;
