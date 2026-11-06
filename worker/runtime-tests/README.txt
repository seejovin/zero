NATIVE CLOUDFLARE WORKER REGRESSION GATE

Runtime: official workerd@1.20261104.1 (workerd 2026-11-04).
Compatibility date: 2026-11-04.

Run after building the actual production ESM Worker:

  node verify.mjs /absolute/path/to/workerd /absolute/path/to/dist/server/index.js

The command uses the exact supplied Worker bundle in real workerd. It does not
replace global fetch or the production relay. A workerd globalOutbound service
provides controlled HTTP responses after native Request/fetch validation.

Checks:
1. Actual production /api/rpc supports Base and Ethereum chain reads.
2. Worker requests use redirect:manual, which is supported by workerd.
3. RPC 307 responses are rejected; the redirect destination receives no request.
4. Blockscout 307 responses yield partial history; its credential is never sent
   to the redirect destination.

The old v0.3.1 bundle fails this gate: native workerd rejects redirect:error
before sending any HTTP request. Node and browser fetch accept that option.

These are deterministic native-runtime checks, not live internet or production
authentication tests. No vault, key, wallet account or transaction is used.
