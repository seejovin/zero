//! Authenticating a shared (never customer-specific) fee publication.
use crate::{
    private_payment::{self, PrivateDomain},
    validation, WalletError,
};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sha3::Keccak256;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFeeSchedule {
    pub domain: PrivateDomain,
    pub schedule_id: String,
    pub fee_units: String,
    pub fee_recipient_address: String,
    pub fee_recipient_commitment: String,
    pub max_native_fee_wei: String,
    pub gas_limit: u64,
    pub valid_from: u64,
    pub valid_until: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedFeeSchedule {
    pub schema_version: u8,
    pub schedule: SharedFeeSchedule,
    pub key_id: String,
    pub signature: String,
}
fn reject() -> WalletError {
    WalletError::Policy {
        reason: "Shared fee schedule authentication failed".into(),
    }
}
pub fn recipient_commitment(address: &str) -> Result<String, WalletError> {
    if !(8..=256).contains(&address.len())
        || !address.starts_with("0zk")
        || !address.bytes().all(|x| x.is_ascii_alphanumeric())
    {
        return Err(reject());
    }
    let mut h = Keccak256::new();
    h.update(b"local-wallet/provider-fee-recipient/v1\0");
    h.update(address.as_bytes());
    Ok(format!("0x{}", hex::encode(h.finalize())))
}
/// The key must come from compiled reviewed release configuration. This is not
/// a TOFU/key-discovery API. SDK address checksum/network checks follow locally.
#[cfg_attr(feature = "native", uniffi::export)]
pub fn private_verify_fee_schedule(
    packet_json: String,
    pinned_public_key: Vec<u8>,
    now: u64,
) -> Result<String, WalletError> {
    if packet_json.len() > 16_384 {
        return Err(reject());
    }
    let p: SignedFeeSchedule = private_payment::decode(&packet_json)?;
    let key_bytes: [u8; 32] = pinned_public_key.try_into().map_err(|_| reject())?;
    let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| reject())?;
    let s = &p.schedule;
    private_payment::validate_domain(&s.domain)?;
    private_payment::canonical_hash(&s.schedule_id)?;
    private_payment::canonical_hash(&s.fee_recipient_commitment)?;
    let native = validation::decimal(&s.max_native_fee_wei)?;
    if p.schema_version != 1
        || p.key_id != format!("0x{}", hex::encode(Sha256::digest(key_bytes)))
        || recipient_commitment(&s.fee_recipient_address)? != s.fee_recipient_commitment
        || validation::decimal(&s.fee_units)?.is_zero()
        || native.is_zero()
        || native > private_payment::MAX_NATIVE_FEE_WEI.into()
        || !(21_000..=private_payment::MAX_GAS_LIMIT).contains(&s.gas_limit)
        || s.valid_from == 0
        || s.valid_until <= s.valid_from
        || s.valid_until > i64::MAX as u64
        || s.valid_until - s.valid_from > private_payment::MAX_FEE_SCHEDULE_SECONDS
        || now < s.valid_from
        || now >= s.valid_until
    {
        return Err(reject());
    }
    let encoded = serde_json::to_string(s).map_err(|_| reject())?;
    let mut signed = b"local-wallet/shared-fee-schedule/v1\0".to_vec();
    signed.extend_from_slice(encoded.as_bytes());
    if validation::fixed_hex(&p.signature, 64)? != p.signature {
        return Err(reject());
    }
    let sig: [u8; 64] = hex::decode(&p.signature[2..])
        .map_err(|_| reject())?
        .try_into()
        .map_err(|_| reject())?;
    key.verify_strict(&signed, &Signature::from_bytes(&sig))
        .map_err(|_| reject())?;
    Ok(encoded)
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn checks_signature_time_domain_and_duplicate_fields() {
        let key = SigningKey::from_bytes(&[9u8; 32]);
        let public = key.verifying_key().to_bytes();
        let address = "0zk1sharedfeeaddress";
        let s = SharedFeeSchedule {
            domain: PrivateDomain {
                protocol: private_payment::CANDIDATE_PROTOCOL.into(),
                chain_id: 42161,
                pool_contract: private_payment::CANDIDATE_POOL.into(),
                token_contract: "0xaf88d065e77c8cc2239327c5edb3a432268e5831".into(),
                token_decimals: 6,
                verifier_digest: format!("0x{}", "11".repeat(32)),
            },
            schedule_id: format!("0x{}", "22".repeat(32)),
            fee_units: "100".into(),
            fee_recipient_address: address.into(),
            fee_recipient_commitment: recipient_commitment(address).unwrap(),
            max_native_fee_wei: "100000".into(),
            gas_limit: 200000,
            valid_from: 100,
            valid_until: 1000,
        };
        let mut bytes = b"local-wallet/shared-fee-schedule/v1\0".to_vec();
        bytes.extend_from_slice(serde_json::to_string(&s).unwrap().as_bytes());
        let packet = SignedFeeSchedule {
            schema_version: 1,
            schedule: s,
            key_id: format!("0x{}", hex::encode(Sha256::digest(public))),
            signature: format!("0x{}", hex::encode(key.sign(&bytes).to_bytes())),
        };
        let json = serde_json::to_string(&packet).unwrap();
        assert!(private_verify_fee_schedule(json.clone(), public.to_vec(), 100).is_ok());
        for t in [99, 1000] {
            assert!(private_verify_fee_schedule(json.clone(), public.to_vec(), t).is_err())
        }
        assert!(private_verify_fee_schedule(
            json.replace("\"100\"", "\"101\""),
            public.to_vec(),
            101
        )
        .is_err());
        assert!(private_verify_fee_schedule(
            json.replace(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1"
            ),
            public.to_vec(),
            101
        )
        .is_err());
    }
}
