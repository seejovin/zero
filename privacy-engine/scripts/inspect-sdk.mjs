/** Read-only qualification report; imports real primitives without networking. */
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {resolve} from 'node:path';
import {candidateInterface,CANDIDATE} from '../src/candidate-calldata.mjs';
import {localKeys,Transaction,TransactNote,Prover} from '../src/protocol.mjs';
import {groth16} from 'snarkjs';
const root=resolve(new URL('..',import.meta.url).pathname);
const versions={};for(const name of ['@railgun-community/engine','@railgun-community/circomlibjs','ethers','snarkjs','@noble/hashes','browserify-aes'])versions[name]=JSON.parse(await readFile(resolve(root,'node_modules',name,'package.json'),'utf8')).version;
if(typeof Transaction!=='function'||typeof TransactNote!=='function'||typeof Prover.formatRailgunInputs!=='function'||typeof groth16.fullProve!=='function'||typeof groth16.verify!=='function')throw Error('Pinned SDK API changed');
const provenance=JSON.parse(await readFile(resolve(root,'vendor/circomlibjs/UPSTREAM.json'),'utf8'));
for(const [path,expected]of Object.entries(provenance.sha256)){
 const bytes=await readFile(resolve(root,'node_modules/@railgun-community/circomlibjs',path));
 if(createHash('sha256').update(bytes).digest('hex')!==expected)throw Error('Vendored cryptographic source differs from upstream');
}
const fixture=await localKeys('11'.repeat(32),'12'.repeat(32));if(fixture.address.length!==127)throw Error('SDK address derivation failed');fixture.destroy();
let audit=null;try{audit=JSON.parse(await readFile(resolve(root,'qualification-audit-current.json'),'utf8')).metadata.vulnerabilities}catch{}
process.stdout.write(JSON.stringify({kind:'actual narrow offline SDK inspection',versions,upstream_cryptographic_source_hashes_verified:true,
 sdk_primitives_imported:true,private_address_derived:true,full_wallet_network_wrapper_loaded:false,global_witness_proof_cache_used:false,
 real_proof_test:'npm run test:protocol -- QUALIFIED_ARTIFACT_DIRECTORY GENERATED_BINDING_DIRECTORY',
 abi:{function:'transact',selector:candidateInterface.getFunction('transact').selector,candidate_chain:Number(CANDIDATE.chainId)},
 dependency_advisory_entries:audit,production_enabled:false,
 remaining_release_gates:['reviewed deployed verifier/code/proxy identity','authenticated shared gateway and independently operated relay','Android build and physical-device qualification','independent security review']},null,2)+'\n');
