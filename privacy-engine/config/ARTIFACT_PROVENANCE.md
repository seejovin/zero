# Published circuit parameters

`upstream-artifact-v2-hashes.json` is the artifact-hash catalog distributed in
`@railgun-community/wallet@11.3.0`. The candidate runtime uses the narrower
`@railgun-community/engine@9.9.1`; it does not load the wallet SDK's network,
subgraph, POI, broadcaster-discovery or global witness-cache wrappers.

The upstream immutable IPFS directory is
`QmUsmnK4PFc7zDp2cmC4wBZxYLjNyRgWfs5GNcJJ2uLcpU`. The selected shapes are one
through eleven inputs and exactly three outputs: recipient, provider fee, and
change or zero-valued private padding. Output order is randomized locally.

The qualification script verifies each decompressed WASM and proving key against
the committed SHA-256 catalog, derives the Groth16 verification key from that
authenticated proving key, and compares a supplied published verification key
with the derived key. No new trusted setup or verification key is generated.
The resulting key hash must also be reviewed against the exact deployed
contract verification key before release. An upstream catalog match alone is
not deployed-contract approval or an independent cryptographic audit.

The first executed positive proof qualification used `01x03`:

| Resource | SHA-256 |
| --- | --- |
| WASM | `72e55865fccca077f443bae9f1e6762a156ea478a137efc1324fb79153050276` |
| Proving key | `477aeca6a8ed3706f572bce0e99a7054e700a4e3c2ee4499d7013a4d7770e0d0` |
| Derived canonical JSON verification key | `b77a9a7b82eb1fde9fdf7184447115552f687c1e4d4aa03b91a2378517dd812a` |

That test used public, unfunded fixture keys and controlled pool observations.
It verified real Groth16 proofs and ciphertexts without claiming that its
synthetic Merkle root exists on mainnet. Actual proving resources remain build
inputs; the service's small verification fixture includes its own provenance.
