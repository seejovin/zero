//! Shared fee publication. A separate pinned Ed25519 key authenticates a common
//! schedule; no customer-specific quote or online arbitrary-message signer.
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use wallet_core::private_payment::{PrivateDomain, MAX_FEE_SCHEDULE_SECONDS};

use crate::model::{bounded_hex, decimal, encode, fixed_hex};
use crate::{AdmissionPolicy, Error, Result};

pub const SIGNING_DOMAIN: &[u8] = b"local-wallet/shared-fee-schedule/v1\0";

/// Field order is part of the v1 signed encoding. Do not sign a generic JSON
/// map. See signing_bytes and the interoperable fixture test.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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

impl SharedFeeSchedule {
    pub fn from_policy(policy: &AdmissionPolicy, fee_recipient_address: String) -> Result<Self> {
        policy.validate()?;
        if recipient_commitment(&fee_recipient_address)? != policy.fee_recipient_commitment {
            return Err(Error::PolicyMismatch);
        }
        Ok(Self {
            domain: policy.domain.clone(),
            schedule_id: policy.schedule_id.clone(),
            fee_units: policy.fee_units.clone(),
            fee_recipient_address,
            fee_recipient_commitment: policy.fee_recipient_commitment.clone(),
            max_native_fee_wei: policy.max_native_fee_wei.clone(),
            gas_limit: policy.gas_limit,
            valid_from: policy.valid_from,
            valid_until: policy.valid_until,
        })
    }

    pub fn signing_bytes(&self) -> Result<Vec<u8>> {
        wallet_core::private_payment::validate_domain(&self.domain)
            .map_err(|_| Error::InvalidPolicy)?;
        fixed_hex(&self.schedule_id, 32)?;
        fixed_hex(&self.fee_recipient_commitment, 32)?;
        if recipient_commitment(&self.fee_recipient_address)? != self.fee_recipient_commitment {
            return Err(Error::VerificationFailed);
        }
        if decimal(&self.fee_units)?.is_zero()
            || decimal(&self.max_native_fee_wei)?.is_zero()
            || decimal(&self.max_native_fee_wei)? > 1_000_000_000_000_000_000u64.into()
            || !(21_000..=10_000_000).contains(&self.gas_limit)
            || self.valid_from == 0
            || self.valid_until <= self.valid_from
            || self.valid_until > i64::MAX as u64
            || self.valid_until - self.valid_from > MAX_FEE_SCHEDULE_SECONDS
        {
            return Err(Error::InvalidPolicy);
        }
        let mut bytes = SIGNING_DOMAIN.to_vec();
        bytes.extend_from_slice(encode(self)?.as_bytes());
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedFeeSchedule {
    pub schema_version: u8,
    pub schedule: SharedFeeSchedule,
    pub key_id: String,
    pub signature: String,
}

impl SignedFeeSchedule {
    /// Offline publication only. This key signs the closed shared fee schema,
    /// never an arbitrary message or a blockchain transaction. Its public key
    /// must be independently distributed in the reviewed wallet release.
    pub fn sign_policy(
        policy: &AdmissionPolicy,
        recipient_address: String,
        source: &crate::secrets::SecretSource,
    ) -> Result<Self> {
        let schedule = SharedFeeSchedule::from_policy(policy, recipient_address)?;
        let bytes = schedule.signing_bytes()?;
        let secret = source.read()?;
        let key = SigningKey::from_bytes(&secret);
        let public = key.verifying_key().to_bytes();
        let packet = Self {
            schema_version: 1,
            schedule,
            key_id: format!("0x{}", hex::encode(Sha256::digest(public))),
            signature: format!("0x{}", hex::encode(key.sign(&bytes).to_bytes())),
        };
        packet.verify(&public)?;
        Ok(packet)
    }

    pub fn parse(input: &str) -> Result<Self> {
        if input.len() > 16_384 {
            return Err(Error::InvalidEncoding);
        }
        crate::model::decode(input)
    }

    pub fn verify(&self, pinned_public_key: &[u8; 32]) -> Result<()> {
        let key_id = format!("0x{}", hex::encode(Sha256::digest(pinned_public_key)));
        if self.schema_version != 1 || self.key_id != key_id {
            return Err(Error::VerificationFailed);
        }
        let key =
            VerifyingKey::from_bytes(pinned_public_key).map_err(|_| Error::VerificationFailed)?;
        let bytes: [u8; 64] = bounded_hex(&self.signature, 64)?
            .try_into()
            .map_err(|_| Error::VerificationFailed)?;
        key.verify_strict(
            &self.schedule.signing_bytes()?,
            &Signature::from_bytes(&bytes),
        )
        .map_err(|_| Error::VerificationFailed)
    }

    pub fn verify_policy(
        &self,
        pinned_public_key: &[u8; 32],
        policy: &AdmissionPolicy,
    ) -> Result<()> {
        self.verify(pinned_public_key)?;
        if self.schedule
            != SharedFeeSchedule::from_policy(policy, self.schedule.fee_recipient_address.clone())?
        {
            return Err(Error::PolicyMismatch);
        }
        Ok(())
    }
}

pub fn recipient_commitment(address: &str) -> Result<String> {
    // Full Bech32/network/address validation is also performed by the selected
    // protocol SDK. This layer validates the exact signed commitment binding.
    if !(8..=256).contains(&address.len())
        || !address.starts_with("0zk")
        || !address.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(Error::InvalidPolicy);
    }
    let mut hash = Keccak256::new();
    hash.update(b"local-wallet/provider-fee-recipient/v1\0");
    hash.update(address.as_bytes());
    Ok(format!("0x{}", hex::encode(hash.finalize())))
}
