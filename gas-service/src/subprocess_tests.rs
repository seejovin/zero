//! Fixed verifier process integration. The success fixture below is deliberately
//! a cfg(test) schema echo, NEVER a proof verifier. The actual pinned SDK verifier
//! is separately invoked and must reject the invalid ABI fixture.
use super::*;
use crate::proof_process::{NodeProofVerifier, NodeVerifierConfig};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::OnceLock};

fn hash_file(path: &Path) -> String {
    format!(
        "0x{}",
        hex::encode(Sha256::digest(std::fs::read(path).unwrap()))
    )
}
fn node() -> (PathBuf, String) {
    static NODE: OnceLock<(PathBuf, String)> = OnceLock::new();
    NODE.get_or_init(|| {
        let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("node"))
            .find(|p| p.is_file())
            .expect("Node must be installed for verifier process integration tests");
        let path = path.canonicalize().unwrap();
        let hash = hash_file(&path);
        (path, hash)
    })
    .clone()
}
fn private_file(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}
fn config(script: PathBuf, operator_config: PathBuf) -> NodeVerifierConfig {
    let (node_executable, node_sha256) = node();
    NodeVerifierConfig {
        node_executable,
        node_sha256,
        script_sha256: hash_file(&script),
        operator_config_sha256: hash_file(&operator_config),
        script,
        operator_config,
        additional_native_fee_cap_wei: "0".into(),
    }
}

#[test]
fn subprocess_contract_binds_fields_rejects_output_leaks_and_changed_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("TEST-ONLY-schema-echo.mjs");
    let operator = temp.path().join("TEST-ONLY-config.json");
    // No production mode, flag or source includes this program.
    private_file(
        &script,
        r#"
import {readFile} from 'node:fs/promises';
if(process.argv.length!==4||process.argv[2]!=='--config'||Object.keys(process.env).some(k=>k!=='LANG'))process.exit(1);
let input='';for await(const bytes of process.stdin)input+=bytes;
const {envelope:e}=JSON.parse(input),{mode}=JSON.parse(await readFile(process.argv[3],'utf8'));
const reply={schema_version:1,operation_digest:e.operation_digest,tree_number:e.tree_number,call_data:e.call_data,fee_output_commitment:e.fee.fee_output_commitment,additional_native_fee_cap_wei:'0'};
if(mode==='wrong-tree')reply.tree_number+=1;
if(mode==='leak')reply.recipient='must-not-be-accepted';
if(mode==='too-large'){process.stdout.write('x'.repeat(300001));}else process.stdout.write(JSON.stringify(reply));
"#,
    );
    let (_, env, _) = crate::rpc_tests::abi_envelope();
    for mode in ["success", "wrong-tree", "leak", "too-large"] {
        private_file(&operator, &serde_json::json!({"mode":mode}).to_string());
        let verifier = NodeProofVerifier::new(config(script.clone(), operator.clone())).unwrap();
        if mode == "success" {
            assert_eq!(
                verifier.verify(&env).unwrap().operation_digest,
                env.operation_digest
            );
        } else {
            assert_eq!(verifier.verify(&env), Err(Error::VerificationFailed));
        }
    }
    private_file(&operator, r#"{"mode":"success"}"#);
    let verifier = NodeProofVerifier::new(config(script.clone(), operator.clone())).unwrap();
    private_file(
        &script,
        "throw new Error('replacement must never execute');",
    );
    assert_eq!(verifier.verify(&env), Err(Error::VerificationFailed));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&operator, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(NodeProofVerifier::new(config(script, operator)).is_err());
    }
}

#[test]
fn actual_sdk_provider_verifier_rejects_invalid_proof_without_admission_or_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let (_, env, _) = crate::rpc_tests::abi_envelope();
    let vkey = temp.path().join("INVALID-verification-key.json");
    // Shape-correct outer artifact label, unusable curve points. This reaches the
    // actual snarkjs verifier and cannot be mistaken for a production circuit.
    private_file(&vkey,&serde_json::json!({"protocol":"groth16","curve":"bn128","nPublic":6,"IC":vec![vec!["0","0","1"];7]}).to_string());
    let operator = temp.path().join("TEST-ONLY-provider.json");
    private_file(&operator,&serde_json::json!({"artifact_directory":".","verifier_digest":env.domain.verifier_digest,"circuits":{"02x02":{"vkey":{"path":"INVALID-verification-key.json","sha256":hash_file(&vkey)[2..]}}},"fee_recipient_address":crate::tests::FEE_ADDRESS,"fee_viewing_private_key_hex":"00".repeat(32),"additional_native_fee_cap_wei":"0"}).to_string());
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../privacy-engine/scripts/provider-verifier.mjs")
        .canonicalize()
        .unwrap();
    let verifier = Arc::new(NodeProofVerifier::new(config(script, operator)).unwrap());
    assert_eq!(verifier.verify(&env), Err(Error::VerificationFailed));
    // Even an injected real verifier cannot open the normal economic gates.
    let service = GasService::open(
        &temp.path().join("journal"),
        crate::rpc_tests::abi_envelope().0,
        verifier,
        Arc::new(crate::tests::FixtureChain),
    )
    .unwrap();
    assert_eq!(
        service.admit(&env.operation_digest, &encode(&env).unwrap(), 1500),
        Err(Error::ReleaseUnavailable)
    );
    assert_eq!(service.summary().unwrap().jobs, 0);
}

#[test]
fn actual_sdk_provider_verifier_accepts_genuine_unfunded_proof_and_rejects_fee_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let env = wallet_core::private_payment::validate_envelope_json(include_str!(
        "../fixtures/genuine-proof/unfunded-envelope.json"
    ))
    .unwrap();
    private_file(
        &temp.path().join("01x03.vkey.json"),
        include_str!("../fixtures/genuine-proof/01x03.vkey.json"),
    );
    let operator = temp.path().join("PUBLIC-TEST-KEYS-only.json");
    private_file(
        &operator,
        include_str!("../fixtures/genuine-proof/TEST-ONLY-provider-config.json"),
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../privacy-engine/scripts/provider-verifier.mjs")
        .canonicalize()
        .unwrap();
    let verifier = NodeProofVerifier::new(config(script, operator)).unwrap();
    let verified = verifier.verify(&env).unwrap();
    assert_eq!(verified.operation_digest, env.operation_digest);
    assert_eq!(verified.tree_number, u64::from(env.tree_number));
    assert_eq!(
        verified.fee_output_commitment,
        env.fee.fee_output_commitment
    );
    assert_eq!(verified.additional_native_fee_cap_wei, "0");
    let mut changed = env;
    changed.fee.fee_units = (changed.fee.fee_units.parse::<u64>().unwrap() + 1).to_string();
    changed.operation_digest = wallet_core::private_payment::envelope_digest(&changed).unwrap();
    // The Groth16 proof remains genuine, but the provider's decrypted fee no
    // longer equals the advertised fee. Hash/shape checks alone cannot catch it.
    assert_eq!(verifier.verify(&changed), Err(Error::VerificationFailed));
}
