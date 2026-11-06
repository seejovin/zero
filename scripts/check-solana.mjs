/** Independent wire, instruction, ATA, derivation and Ed25519 checks.
 * All inputs are public offline fixtures. Never contacts a blockchain endpoint.
 *
 * Install test-only packages outside the app:
 * npm install --prefix /tmp/local-wallet-solana-check --no-audit --no-fund \
 *   @solana/web3.js@1.98.4 @solana/spl-token@0.4.15 ed25519-hd-key@2.0.0
 * cargo run --locked --quiet --example solana_differential_vectors > /tmp/solana-vectors.json
 * SOLANA_DIFF_MODULES=/tmp/local-wallet-solana-check/node_modules \
 *   node scripts/check-solana.mjs /tmp/solana-vectors.json
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pbkdf2Sync } from 'node:crypto';

const require = createRequire(resolve(process.env.SOLANA_DIFF_MODULES || 'node_modules', '__offline_verifier__.cjs'));
const { PublicKey, Keypair, Transaction, Message, ComputeBudgetProgram, SystemProgram } = require('@solana/web3.js');
const { getAssociatedTokenAddressSync, createAssociatedTokenAccountIdempotentInstruction, createTransferCheckedInstruction, TOKEN_PROGRAM_ID } = require('@solana/spl-token');
const { derivePath } = require('ed25519-hd-key');

const source = JSON.parse(readFileSync(process.argv[2] || '/tmp/solana-vectors.json', 'utf8'));
assert.equal(source.fixture, 'PUBLIC UNFUNDED OFFLINE ONLY');
assert.equal(source.derivation_path, "m/44'/501'/0'/0'");
const seed = pbkdf2Sync(source.phrase.normalize('NFKD'), 'mnemonic', 2048, 64, 'sha512');
const derived = derivePath(source.derivation_path, seed.toString('hex'));
const signer = Keypair.fromSeed(derived.key);
assert.equal(signer.publicKey.toBase58(), source.owner);
const mints = {
  USDC: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
  EURC: 'HzwqbKZw8HxMN6bF2yFZNrht3c2iXXzpKcFu7uBEDKtr',
  USDT: 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB',
};
let checked = 0;
for (const { prepared, signed } of source.vectors) {
  const request = prepared.request;
  const from = new PublicKey(request.from), to = new PublicKey(request.to);
  const independent = [
    ComputeBudgetProgram.setComputeUnitLimit({ units: request.compute_unit_limit }),
    ComputeBudgetProgram.setComputeUnitPrice({ microLamports: BigInt(request.compute_unit_price_micro_lamports) }),
  ];
  if (request.asset === 'SOL') {
    independent.push(SystemProgram.transfer({ fromPubkey: from, toPubkey: to, lamports: BigInt(prepared.amount_units) }));
  } else {
    const mint = new PublicKey(mints[request.asset]);
    const destination = getAssociatedTokenAddressSync(mint, to);
    assert.equal(destination.toBase58(), prepared.destination_token_account);
    if (request.create_destination_ata) independent.push(createAssociatedTokenAccountIdempotentInstruction(from, destination, to, mint));
    let total = 0n;
    for (const account of request.source_accounts) {
      const amount = BigInt(account.amount_units); total += amount;
      independent.push(createTransferCheckedInstruction(new PublicKey(account.address), mint, destination, from, amount, 6, [], TOKEN_PROGRAM_ID));
    }
    assert.equal(total.toString(), prepared.amount_units);
  }
  const messageBytes = Buffer.from(prepared.unsigned_transaction, 'base64');
  const decoded = Message.from(messageBytes);
  assert.equal(decoded.header.numRequiredSignatures, 1);
  assert.equal(decoded.accountKeys[0].toBase58(), from.toBase58());
  assert.equal(decoded.recentBlockhash, request.recent_blockhash);
  assert.deepEqual(Buffer.from(decoded.serialize()), messageBytes);
  const raw = Buffer.from(signed.raw_transaction, 'base64');
  const transaction = Transaction.from(raw);
  assert.equal(transaction.instructions.length, independent.length);
  // Global Solana account writability is the OR of every instruction's flags,
  // including the fee payer. Compare program IDs, exact data and ordered keys;
  // the account header below independently verifies the global permissions.
  const permissions = new Map([[from.toBase58(), { signer: true, writable: true }]]);
  for (const instruction of independent) {
    permissions.set(instruction.programId.toBase58(), permissions.get(instruction.programId.toBase58()) || { signer: false, writable: false });
    for (const key of instruction.keys) {
      const prior = permissions.get(key.pubkey.toBase58()) || { signer: false, writable: false };
      permissions.set(key.pubkey.toBase58(), { signer: prior.signer || key.isSigner, writable: prior.writable || key.isWritable });
    }
  }
  assert.equal(permissions.size, decoded.accountKeys.length);
  decoded.accountKeys.forEach((key, index) => {
    const flags = permissions.get(key.toBase58()); assert.ok(flags);
    assert.equal(decoded.isAccountSigner(index), flags.signer);
    assert.equal(decoded.isAccountWritable(index), flags.writable);
  });
  independent.forEach((expected, index) => {
    const actual = transaction.instructions[index];
    assert.equal(actual.programId.toBase58(), expected.programId.toBase58());
    assert.deepEqual(actual.data, expected.data);
    assert.deepEqual(actual.keys.map(k => k.pubkey.toBase58()), expected.keys.map(k => k.pubkey.toBase58()));
  });
  assert.ok(transaction.verifySignatures());
  assert.deepEqual(transaction.serialize({ requireAllSignatures: true, verifySignatures: true }), raw);
  // Re-sign the decoded message with an independently derived JS Ed25519 signer.
  transaction.sign(signer);
  assert.deepEqual(transaction.serialize({ requireAllSignatures: true, verifySignatures: true }), raw);
  assert.deepEqual(raw.subarray(65), messageBytes);
  assert.equal(raw[0], 1); assert.ok(raw.length <= 1232);
  checked++;
}
signer.secretKey.fill(0); derived.key.fill(0); seed.fill(0);
console.log(`Solana differential checks passed: ${checked} public offline vectors; independent derivation, ATA, instructions, message, wire bytes and Ed25519 signatures.`);
