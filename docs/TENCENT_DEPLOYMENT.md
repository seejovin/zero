# zerø — Tencent gas service deployment

**Tencent hosts the gas gateway/broadcaster; it is not required to compile or install the Android APK.** The current source can run a bounded OHTTP diagnostics service, but production private sponsorship remains disabled in the core, service and Android release configuration. A running server, valid configuration or successful APK build does not change that status. See [PRIVATE_GAS_UPGRADE.md](PRIVATE_GAS_UPGRADE.md).

This runbook prepares an isolated operator host. It has not been executed against your Tencent account. Commands that start services are conditional on receiving the reviewed configuration described below. No funded treasury, production endpoint, independent relay or verified contract-pin bundle is included in this repository.

## 1. Create the operator instance

In Tencent **CVM**, create a dedicated Ubuntu **24.04 LTS, x86-64** instance. A starting engineering allowance is **2 vCPU, 4 GiB RAM, 40 GiB SSD** for a small staging service; this is a sizing assumption, not a measured capacity claim. Build large native dependencies on the CI runner if this machine is too small. Use an SSH key and a separate service account. Keep the gateway separate from the existing dakim web application.

Use a custom security group, with matching IPv6 restrictions if IPv6 is enabled:

| Direction | Rule | Purpose |
| --- | --- | --- |
| Inbound | TCP 22 from your administration IP/CIDR only | SSH |
| Inbound | TCP 443 from the independent relay's agreed egress IPs; initially your staging client only | OHTTP over TLS |
| Inbound | TCP 80 only if the selected certificate renewal method needs HTTP-01 | Certificate validation |
| Inbound | No external 8080/8081 rule | Both Rust listeners stay on loopback |
| Outbound | HTTPS to the two selected RPC providers and approved maintenance destinations; required DNS/time services | Chain evidence, updates and clock accuracy |

Do not select Tencent's unrestricted-port template. Tencent documents both custom rules and templates that open more ports than this service needs. [Tencent security groups](https://www.tencentcloud.com/document/product/213/12452)

Security-group destinations use IP addresses/CIDRs, not RPC hostnames. Use the selected providers' maintained IP ranges or a separately configured egress control that understands hostnames; account for certificate renewal and dependency installation when defining the approved maintenance destinations. A provider changing its addresses can otherwise interrupt the service.

Connect from your own computer using the instance's actual SSH key and public IP:

```sh
ssh -i /absolute/path/to/your-key.pem ubuntu@YOUR_CVM_PUBLIC_IP
```

The Ubuntu image normally uses `ubuntu`; use the account shown for your actual image. [Tencent SSH instructions](https://www.tencentcloud.com/document/product/213/32501)

## 2. Use an existing subdomain

If you control `dakim-test.com` DNS, a new name such as **`gas.dakim-test.com`** can point to this instance using an A record. This does not require another domain registration. The name is a proposal, not an endpoint already created. The app's pinned OHTTP authority and gateway configuration must use the final reviewed hostname. Add an AAAA record only after separately configuring and testing an IPv6 TLS listener and matching security-group rules; the supplied NGINX example listens on IPv4.

Install a publicly trusted TLS certificate and automate renewal. DNS-01 permits keeping port 80 closed; HTTP-01 requires port 80. Keep broad DNS credentials off the gas host; use narrowly scoped validation credentials or a separate certificate issuer. Do not use a self-signed certificate or disable certificate verification. [Let's Encrypt validation methods](https://letsencrypt.org/docs/challenge-types/)

A proxy/CDN is an additional observer. For the first deployment, use direct DNS and the dedicated ingress below, then verify its logging behavior. An IP-only deployment is not the configured release design.

## 3. Compile and inspect the service

Use a clean build account or CI runner with the official Rust toolchain and **Node.js 24** installed. The repository pins Rust **1.99.1** and dependencies. Install the OS build prerequisites on Ubuntu:

```sh
sudo apt-get update
sudo apt-get install --yes build-essential pkg-config ca-certificates git openssl nginx curl
git clone https://github.com/seejovin/zero.git zero
cd zero
git checkout --detach REVIEWED_COMMIT_SHA
rustup toolchain install 1.99.1 --profile minimal
npm --prefix privacy-engine ci --ignore-scripts --no-fund --no-audit
cargo +1.99.1 test --locked -p wallet_gas_service
cargo +1.99.1 build --locked --release -p wallet_gas_service --bins
./target/release/wallet-gas-service capabilities
```

Replace `REVIEWED_COMMIT_SHA` with the exact release commit. Authenticate to a private GitHub repository through your normal credential mechanism; do not put a token in the clone URL. Obtain Rust/Node installers from their [official Rust](https://www.rust-lang.org/tools/install) and [official Node](https://nodejs.org/en/download) channels, verify the release, and record the installed versions. No customer wallet keys are involved in building.

At this revision, `capabilities` must report **`production_sponsorship_available: false`**. That is the expected staging result. The service component version may differ from the Android package version.

Install the reviewed binaries, the pinned Node binary, and the complete `privacy-engine` directory including its locked `node_modules`. The verifier imports adjacent source modules; copying only `provider-verifier.mjs` is insufficient. Keep runtime code root-owned and not writable by the service user. The configured Node path must be a regular file, not a symlink.

Suggested first-install layout:

```text
/opt/zero-gas/bin/wallet-gas-service
/opt/zero-gas/bin/wallet-ohttp-relay
/opt/zero-gas/node/bin/node
/opt/zero-gas/privacy-engine/             # source + locked runtime dependencies
/opt/zero-gas/artifacts/                  # qualified verification-key files
/etc/zero-gas/operator.json
/etc/zero-gas/provider-verifier.json
/etc/zero-gas/gateway-hpke-seed.hex
/var/lib/zero-gas/operator.sqlite
```

Create the service account and directories once:

```sh
sudo useradd --system --home-dir /var/lib/zero-gas --shell /usr/sbin/nologin zero-gas
sudo install -d -o root -g root -m 0755 /opt/zero-gas/bin /opt/zero-gas/node/bin
sudo install -d -o root -g zero-gas -m 0750 /etc/zero-gas
sudo install -d -o zero-gas -g zero-gas -m 0700 /var/lib/zero-gas
sudo install -o root -g root -m 0755 target/release/wallet-gas-service /opt/zero-gas/bin/
sudo install -o root -g root -m 0755 target/release/wallet-ohttp-relay /opt/zero-gas/bin/
```

Complete the first install from the same reviewed checkout. `ZERO_NODE_BINARY` below must identify the **real binary** in the verified Node distribution; the copy becomes a regular, root-owned file at the configured runtime path. These commands stop on any failure and refuse to replace an existing engine directory so an update cannot leave stale dependencies mixed into a release:

```sh
(
    set -eu
    ZERO_NODE_BINARY=/absolute/path/to/verified-node-24/bin/node
    test -f "$ZERO_NODE_BINARY"
    test ! -L "$ZERO_NODE_BINARY"
    sudo test ! -e /opt/zero-gas/privacy-engine
    sudo test ! -L /opt/zero-gas/privacy-engine
    sudo install -o root -g root -m 0755 "$ZERO_NODE_BINARY" /opt/zero-gas/node/bin/node
    sudo cp -R --no-preserve=ownership privacy-engine /opt/zero-gas/privacy-engine
    sudo chown -R root:root /opt/zero-gas/privacy-engine
    sudo chmod -R go-w /opt/zero-gas/privacy-engine
    sudo install -d -o root -g root -m 0755 /opt/zero-gas/artifacts
)
```

Stop if any check fails. For a later update, stage a fresh release directory and verify its complete files before switching the service; retain the same operator journal and keys. The journal is never part of the code directory.

The operator journal is one durable local SQLite database. Run **one active operator instance** per treasury/journal. Do not clone its treasury into independently writable journals or autoscale this process as if it were stateless.

## 4. Prepare reviewed configuration and separate keys

Start from [`gas-service/config.example.json`](../gas-service/config.example.json). Its placeholder values are deliberately invalid. Use `/etc/zero-gas` and `/var/lib/zero-gas` paths from this runbook. For transport-only staging, `proof_verifier`, `rpc` and `treasury_secret` may be `null`; a valid signed common fee schedule and policy are still required. That configuration cannot serve pool history or sponsor a transaction.

Before functional qualification, the release bundle must supply:

- The exact Arbitrum One/native-USDC domain, deployment block, current and historical pool/token/proxy/implementation pins, compatible event history, and deployed verifier-key hashes for each supported circuit.
- Two independently operated HTTPS archive RPC providers supporting the required historical calls and complete block receipts.
- The provider's own private fee address and viewing key, qualified circuit manifests, and exact SHA-256 pins for Node, verifier entry script and private verifier configuration. Keep all imported verifier code/dependencies immutable under the same release review.
- A common fee policy with exact integer USDC fees, a validity interval of at most 24 hours, per-operation cumulative gas cap, global pending cap and lifetime spending cap.
- The independently authenticated fee-publisher public key, signed fee packet, gateway HPKE configuration/pin and independent relay URL. These become reviewed app configuration; downloading a public key beside its hash does not authenticate it.

Do not substitute explorer guesses, empty historical pin lists or repository test fixtures for this qualification.

For the offline verifier's exact field layout, use [`provider-verifier.json.example`](../deploy/tencent/provider-verifier.json.example). It deliberately contains invalid placeholders and only one example circuit entry. Add **every reviewed supported shape** from `qualified-circuits.json`; an omitted shape is rejected. Copy the matching public verification-key files into `/opt/zero-gas/artifacts/`, root-owned and not writable by `zero-gas`. The example's `artifact_directory` is `/opt/zero-gas`, so `artifacts/01x03.vkey.json` resolves to `/opt/zero-gas/artifacts/01x03.vkey.json`. The gateway only needs the verification keys; mobile proving parameters are separate client artifacts.

Keep the two hash formats distinct: `circuits.<shape>.vkey.sha256` is exactly 64 lowercase hexadecimal characters **without** `0x`; the Rust `proof_verifier` fields `node_sha256`, `script_sha256` and `operator_config_sha256` require `0x` followed by 64 lowercase hexadecimal characters. Compute the latter from the exact installed Node binary, script and completed verifier configuration. The configuration digest changes when its provider key, address, paths or circuit entries change. Hashing a file does not establish its provenance or deployed-contract correspondence.

| Secret | Handling |
| --- | --- |
| Gateway HPKE seed | Fresh 32-byte CSPRNG value, gateway only |
| Fee-publisher Ed25519 seed | Generate and keep on an offline publication machine; upload only public key and signed schedules |
| Provider private viewing key | Only in the fixed `provider-verifier.json`; reveals the provider's own fee notes |
| Operator ETH spending key | Separate treasury key; no funding or installation in transport-only staging |
| Customer recovery/spending/viewing keys | Never supplied to the operator or relay |

Generate the HPKE seed on the gateway without displaying it or overwriting an existing key:

```sh
sudo bash -c 'set -euo pipefail; set -o noclobber; umask 077
openssl rand -hex 32 > /etc/zero-gas/gateway-hpke-seed.hex
chown zero-gas:zero-gas /etc/zero-gas/gateway-hpke-seed.hex'
```

On the **offline publication machine**, Node's standard Ed25519 key generator can create the publisher seed and public key. These are provider keys, not customer keys. Run in a new private directory; the exclusive file writes refuse to overwrite keys:

```sh
umask 077
node --input-type=module <<'NODE'
import {generateKeyPairSync} from 'node:crypto';
import {writeFileSync} from 'node:fs';
const {privateKey} = generateKeyPairSync('ed25519');
const key = privateKey.export({format: 'jwk'});
writeFileSync('fee-publisher-seed.hex', Buffer.from(key.d, 'base64url').toString('hex') + '\n', {flag: 'wx', mode: 0o600});
writeFileSync('fee-publisher-public-key.hex', '0x' + Buffer.from(key.x, 'base64url').toString('hex') + '\n', {flag: 'wx', mode: 0o600});
NODE
ZERO_GAS_BINARY=/absolute/path/to/reviewed/wallet-gas-service
"$ZERO_GAS_BINARY" sign-fee-schedule policy.json provider-public-address.txt fee-publisher-seed.hex > fee-schedule.json
```

Use the reviewed policy and actual provider public privacy address for that last command. It signs only the closed fee schema. Store the seed backup offline. The runtime needs neither the publisher seed nor customer secrets. Owner-only runtime files must be mode `0600`, owned by `zero-gas`, with a root-owned parent directory; secret values must not be CLI arguments or GitHub artifacts. The private provider viewing-key provisioning and protocol wallet generation still require the reviewed release tooling, not the public fixture wallet.

Install the completed configuration from a private staging directory on the gateway. Keep the publisher seed on the offline machine. For transport-only staging, omit the verifier-config installation and keep `proof_verifier: null` in `operator.json`:

```sh
sudo install -o zero-gas -g zero-gas -m 0600 /absolute/private/path/operator.json /etc/zero-gas/operator.json
# Only for a configured offline verifier, after its provider key and artifacts are reviewed:
sudo install -o zero-gas -g zero-gas -m 0600 /absolute/private/path/provider-verifier.json /etc/zero-gas/provider-verifier.json
```

The example `/etc/wallet-gas` and `/opt/wallet-gas` paths in the general service configuration must all be replaced with this runbook's `/etc/zero-gas` and `/opt/zero-gas` paths; `journal` must be `/var/lib/zero-gas/operator.sqlite`. No private configuration belongs in the checkout. After configuration is complete, run as the actual service user:

```sh
sudo -u zero-gas /opt/zero-gas/bin/wallet-gas-service check-runtime /etc/zero-gas/operator.json
sudo -u zero-gas /opt/zero-gas/bin/wallet-gas-service gateway-public-key /etc/zero-gas/operator.json
```

`check-runtime` validates schemas and the fee signature; it does **not** contact RPC, inspect all executable files, or establish deployed-contract correctness. `gateway-public-key` reads only the HPKE secret and prints public configuration. Preserve the reviewed public pin for the app build.

## 5. Start the staging gateway and HTTPS ingress

Review and install [`zero-gas.service.example`](../deploy/tencent/zero-gas.service.example) as `/etc/systemd/system/zero-gas.service`. It uses a restricted service account, read-only application files, one writable state directory, no core dumps and graceful SIGINT shutdown. It does not enable private sending.

```sh
sudo install -o root -g root -m 0644 deploy/tencent/zero-gas.service.example /etc/systemd/system/zero-gas.service
sudo systemd-analyze verify /etc/systemd/system/zero-gas.service
sudo systemctl daemon-reload
sudo systemctl enable --now zero-gas.service
sudo systemctl status --no-pager zero-gas.service
curl --silent --output /dev/null --write-out '%{http_code}\n' http://127.0.0.1:8080/ohttp
```

That plain GET should return **404** from the gateway: it demonstrates a listener, not successful OHTTP or gas sponsorship. The application accepts only bounded, correctly framed `POST /ohttp` encrypted messages. A valid OHTTP status request and response-decryption test are required before calling transport working; an outer HTTP 200 alone is insufficient.

Use [`zero-gateway.nginx.conf.example`](../deploy/tencent/zero-gateway.nginx.conf.example) on the dedicated gateway host. Replace its hostname/certificate paths and validate with `sudo nginx -t` before reloading. The fragment forwards only permitted headers, bounds body size, disables upstream retries/caching and suppresses its request logs. Review global/default-server/TLS-error logs too; a server fragment cannot control logs written before that virtual host is selected. Confirm no request bodies spill into temporary files at the configured maximum size. The NGINX fragment has not been executed or load-tested here. [NGINX proxy directives](https://nginx.org/en/docs/http/ngx_http_proxy_module.html), [access logging](https://nginx.org/en/docs/http/ngx_http_log_module.html), [error logging](https://nginx.org/en/docs/ngx_core_module.html#error_log)

On a fresh dedicated Ubuntu host, install the **edited** fragment under `/etc/nginx/conf.d/zero-gateway.conf` (which is included inside NGINX's `http` block). Remove only the distribution's default-site symlink if it is still present; keep all unrelated site configurations. The hostname in `server_name`, the forwarded `Host`, `operator.json`'s `authority`, the relay destination and the approved app configuration must agree.

```sh
sudo install -o root -g root -m 0644 /absolute/path/to/edited-zero-gateway.nginx.conf /etc/nginx/conf.d/zero-gateway.conf
if [ -L /etc/nginx/sites-enabled/default ]; then
    sudo unlink /etc/nginx/sites-enabled/default
fi
sudo nginx -t
sudo systemctl reload nginx
```

## 6. Independent relay

The relay must be operated by an independent party under an independently verified logging policy. A second VM or separate Tencent account still controlled by the company does **not** meet this requirement. Otherwise the same operator can correlate the client's network address with the decrypted gateway request.

The independent operator uses [`gas-service/relay.example.json`](../gas-service/relay.example.json), replacing only the fixed gateway destination and using `cors_origin: null` for the Android-only release:

```sh
wallet-ohttp-relay check-config /etc/zero-relay/relay.json
wallet-ohttp-relay serve /etc/zero-relay/relay.json
```

It binds to `127.0.0.1:8081` and needs its own reviewed HTTPS ingress. Its ingress must preserve fixed content length/type and avoid retaining request metadata/bodies. The relay accepts no operator HPKE, viewing or treasury keys. Verify that the company gateway sees the relay as its network peer and receives no client IP, cookie, authentication or tracing headers. Do not configure the APK to call the company gateway directly as a fallback.

## 7. Acceptance and operation

Before changing any compiled release gate, complete deployed-contract/circuit qualification, real Android WebView/device tests, relay/gateway transport tests, restart and unknown-outcome reconciliation, and independent security review. Then perform a separately approved, bounded mainnet acceptance transfer with an isolated operator treasury. Keep the original APK signing identity for the Android update. Hosting and APK signing are separate tasks.

Monitor aggregate availability, process health, resource usage, treasury exposure and invariant failures. Keep request logging, analytics identifiers, body capture and per-user traces absent. Review Tencent flow logs, load balancers, CDN/WAF, agent telemetry and crash dumps as separate data surfaces; do not claim that application log settings eliminate cloud-provider observation.

The operator **does retain data** needed for transaction correctness: opaque operation/proof/ciphertext records, nullifier and nonce claims, outer transaction hashes, fees, timing and accounting evidence. It does not receive customer recovery keys or a plaintext private recipient/principal through the intended interface. ZK/OHTTP do not eliminate traffic correlation or relay/gateway collusion. Preserve the journal's permanent replay claims; never delete or edit rows to retry a payment. Use coherent SQLite backups with access-controlled encrypted retention and verify `wallet-gas-service inspect DATABASE` on a restored copy. That inspection is an integrity/accounting check, not permission to broadcast from a rolled-back treasury journal.

**Immediate handoff:** provide the instance's region, public IP, OS and intended gateway hostname, plus the selected independent relay and RPC providers. Keep SSH private keys, wallet seeds and operator secrets out of chat. No additional domain is needed if an existing domain's DNS is under your control. Service activation remains blocked until the reviewed configuration and acceptance evidence above exist.
