// Run after scripts/build-wasm.sh. Public fixtures only; no RPC or broadcasting.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import * as core from '../../build/wasm/wallet_core.js';

await core.default({ module_or_path: await readFile(new URL('../../build/wasm/wallet_core_bg.wasm', import.meta.url)) });
assert.deepEqual(JSON.parse(core.asset_registry()).networks.map(n => n.chain_id), [1,8453,10,42161]);
const key = core.derive_vault_key('public-password-fixture', Uint8Array.from({length:16},(_,i)=>i));
assert.equal(Buffer.from(key).toString('hex'),'63304e0f639a70751dfba7f0ab5ce77ffc4ae30bd8ce59df883cf5b9c3d3a7c8');
const fixture = 'test test test test test test test test test test test junk';
const wallet = JSON.parse(core.restore_wallet(fixture,key));
assert.equal(wallet.address,'0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266');
assert.equal(core.wallet_address(wallet.vault_hex,key),wallet.address);
assert.equal(core.reveal_mnemonic(wallet.vault_hex,key),fixture);
const generated = JSON.parse(core.generate_wallet(key));
assert.equal(generated.mnemonic.split(' ').length,24);
assert.equal(JSON.parse(core.restore_wallet(generated.mnemonic,key)).address,generated.address);
generated.mnemonic = '';
const block = `0x${'12'.repeat(32)}`;
for (const chain_id of [1,8453,10,42161]) {
  const separateFees = chain_id === 8453 || chain_id === 10;
  const prepared = core.prepare_payment(JSON.stringify({intent_id:`public-fixture-${chain_id}`,
    chain_id,from:wallet.address,to:'0x70997970C51812dc3A010C7d01b50e0d17dc79C8',asset:'USDC',
    amount:'1.234567',nonce:3,gas_limit:80000,max_fee_per_gas:'2000000000',max_priority_fee_per_gas:'1000000000',
    l1_fee_estimate_wei:separateFees?'1000':'0',operator_fee_estimate_wei:separateFees?'100':'0'}));
  const review = JSON.parse(prepared);
  assert.equal(core.validate_prepared_json(prepared),prepared);
  const signed = core.sign_payment(wallet.vault_hex,key,prepared,review.approval_digest);
  assert.equal(core.validate_signed_for_prepared(prepared,signed),signed);
  const payment = JSON.parse(signed);
  const actual = JSON.parse(core.inspect_signed(payment.raw_transaction));
  assert.equal(actual.chain_id,chain_id);
  assert.equal(actual.hash,payment.hash);
  const receipt = {transactionHash:payment.hash,blockHash:block,blockNumber:'0x64',from:wallet.address,
    to:actual.transaction_to,status:'0x1',gasUsed:'0xc350',effectiveGasPrice:'0x59682f00',type:'0x2',contractAddress:null,
    logs:[{address:actual.transaction_to,transactionHash:payment.hash,blockHash:block,blockNumber:'0x64',logIndex:'0x0',removed:false,
      topics:['0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef',
        `0x${wallet.address.slice(2).toLowerCase().padStart(64,'0')}`,`0x${payment.to.slice(2).toLowerCase().padStart(64,'0')}`],
      data:`0x${BigInt(payment.amount_units).toString(16).padStart(64,'0')}`} ]};
  if (separateFees) { receipt.l1Fee='0x3e8';receipt.operatorFee='0x64'; }
  const checked = JSON.parse(core.validate_receipt(signed,JSON.stringify(receipt),block,100n));
  assert.equal(checked.execution,'success');assert.equal(checked.inclusion,'finalized');
  assert.equal(checked.actual_fee_wei,separateFees?'75000000001100':'75000000000000');
}
key.fill(0);
console.log('PASS: actual WASM wallet entropy, vault, independent Argon2id vector, four-mainnet USDC signing, shared immutable validation and receipt finality. No network calls.');
