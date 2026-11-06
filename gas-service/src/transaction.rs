//! Parse and verify the operator's signed outer transaction without accepting a
//! caller-supplied sender/hash or implementing a generic signing service.
use k256::ecdsa::{signature::hazmat::PrehashVerifier, RecoveryId, Signature, VerifyingKey};
use primitive_types::U256;
use rlp::{Rlp, RlpStream};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::model::{bounded_hex, decimal, AdmissionPolicy, VerifiedOperation, MAX_CALLDATA};
use crate::{Error, Result};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedOuter {
    pub raw_transaction: String,
    pub transaction_hash: String,
    pub nonce: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: String,
}

impl std::fmt::Debug for SignedOuter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOuter")
            .field("transaction_hash", &self.transaction_hash)
            .field("nonce", &self.nonce)
            .field("raw_transaction", &"[REDACTED]")
            .finish()
    }
}

fn data<'a>(rlp: &Rlp<'a>, index: usize) -> Result<&'a [u8]> {
    let item = rlp.at(index).map_err(|_| Error::InvalidOuterTransaction)?;
    if !item.is_data() {
        return Err(Error::InvalidOuterTransaction);
    }
    item.data().map_err(|_| Error::InvalidOuterTransaction)
}

fn integer(rlp: &Rlp<'_>, index: usize) -> Result<U256> {
    let bytes = data(rlp, index)?;
    if bytes.len() > 32 || bytes.first() == Some(&0) {
        return Err(Error::InvalidOuterTransaction);
    }
    Ok(U256::from_big_endian(bytes))
}

fn u64_integer(rlp: &Rlp<'_>, index: usize) -> Result<u64> {
    let value = integer(rlp, index)?;
    if value > U256::from(u64::MAX) {
        return Err(Error::InvalidOuterTransaction);
    }
    Ok(value.low_u64())
}

fn append_integer(stream: &mut RlpStream, value: U256) {
    if value.is_zero() {
        stream.append_empty_data();
    } else {
        let bytes = value.to_big_endian();
        let offset = bytes.iter().position(|b| *b != 0).unwrap_or(32);
        stream.append(&&bytes[offset..]);
    }
}

#[derive(Clone)]
struct OuterFields {
    chain_id: u64,
    nonce: u64,
    priority: U256,
    max_fee: U256,
    gas_limit: u64,
    to: [u8; 20],
    call_data: Vec<u8>,
}

impl OuterFields {
    fn encode(&self, signature: Option<(&Signature, RecoveryId)>) -> Vec<u8> {
        let mut rlp = RlpStream::new_list(if signature.is_some() { 12 } else { 9 });
        append_integer(&mut rlp, self.chain_id.into());
        append_integer(&mut rlp, self.nonce.into());
        append_integer(&mut rlp, self.priority);
        append_integer(&mut rlp, self.max_fee);
        append_integer(&mut rlp, self.gas_limit.into());
        rlp.append(&self.to.as_slice());
        append_integer(&mut rlp, U256::zero());
        rlp.append(&self.call_data.as_slice());
        rlp.begin_list(0);
        if let Some((signature, recovery)) = signature {
            append_integer(&mut rlp, recovery.to_byte().into());
            let bytes = signature.to_bytes();
            append_integer(&mut rlp, U256::from_big_endian(&bytes[..32]));
            append_integer(&mut rlp, U256::from_big_endian(&bytes[32..]));
        }
        let mut bytes = vec![2];
        bytes.extend(rlp.out());
        bytes
    }
}

pub(crate) fn inspect_outer(
    raw_transaction: &str,
    policy: &AdmissionPolicy,
    verified: &VerifiedOperation,
) -> Result<SignedOuter> {
    let raw = bounded_hex(raw_transaction, MAX_CALLDATA + 512)?;
    if raw.len() < 2 || raw[0] != 2 {
        return Err(Error::InvalidOuterTransaction);
    }
    let rlp = Rlp::new(&raw[1..]);
    let info = rlp
        .payload_info()
        .map_err(|_| Error::InvalidOuterTransaction)?;
    if info.header_len.checked_add(info.value_len) != Some(raw.len() - 1)
        || !rlp.is_list()
        || rlp
            .item_count()
            .map_err(|_| Error::InvalidOuterTransaction)?
            != 12
        || !integer(&rlp, 6)?.is_zero()
    {
        return Err(Error::InvalidOuterTransaction);
    }
    let access = rlp.at(8).map_err(|_| Error::InvalidOuterTransaction)?;
    if !access.is_list()
        || access
            .item_count()
            .map_err(|_| Error::InvalidOuterTransaction)?
            != 0
    {
        return Err(Error::InvalidOuterTransaction);
    }
    let fields = OuterFields {
        chain_id: u64_integer(&rlp, 0)?,
        nonce: u64_integer(&rlp, 1)?,
        priority: integer(&rlp, 2)?,
        max_fee: integer(&rlp, 3)?,
        gas_limit: u64_integer(&rlp, 4)?,
        to: data(&rlp, 5)?
            .try_into()
            .map_err(|_| Error::InvalidOuterTransaction)?,
        call_data: data(&rlp, 7)?.to_vec(),
    };
    let cap = fields
        .max_fee
        .checked_mul(U256::from(fields.gas_limit))
        .and_then(|cost| cost.checked_add(decimal(&verified.additional_native_fee_cap_wei).ok()?))
        .ok_or(Error::Arithmetic)?;
    if fields.chain_id != policy.domain.chain_id
        || fields.nonce == u64::MAX
        || fields.gas_limit != policy.gas_limit
        || fields.priority > fields.max_fee
        || fields.max_fee.is_zero()
        || cap > decimal(&policy.max_native_fee_wei)?
        || fields.to.as_slice() != bounded_hex(&policy.domain.pool_contract, 20)?.as_slice()
        || fields.call_data != bounded_hex(&verified.call_data, MAX_CALLDATA)?
    {
        return Err(Error::InvalidOuterTransaction);
    }
    let parity = u64_integer(&rlp, 9)?;
    if parity > 1 {
        return Err(Error::InvalidOuterTransaction);
    }
    let signature = Signature::from_scalars(
        integer(&rlp, 10)?.to_big_endian(),
        integer(&rlp, 11)?.to_big_endian(),
    )
    .map_err(|_| Error::InvalidOuterTransaction)?;
    if signature.normalize_s() != signature {
        return Err(Error::InvalidOuterTransaction);
    }
    let recovery =
        RecoveryId::try_from(parity as u8).map_err(|_| Error::InvalidOuterTransaction)?;
    if fields.encode(Some((&signature, recovery))) != raw {
        return Err(Error::InvalidOuterTransaction);
    }
    let signing_digest = Keccak256::digest(fields.encode(None));
    let key = VerifyingKey::recover_from_prehash(&signing_digest, &signature, recovery)
        .map_err(|_| Error::InvalidOuterTransaction)?;
    key.verify_prehash(&signing_digest, &signature)
        .map_err(|_| Error::InvalidOuterTransaction)?;
    let key_bytes = key.to_sec1_point(false);
    let address = format!(
        "0x{}",
        hex::encode(&Keccak256::digest(&key_bytes.as_bytes()[1..])[12..])
    );
    if address != policy.broadcaster_address {
        return Err(Error::InvalidOuterTransaction);
    }
    Ok(SignedOuter {
        raw_transaction: raw_transaction.to_string(),
        transaction_hash: format!("0x{}", hex::encode(Keccak256::digest(raw))),
        nonce: fields.nonce,
        gas_limit: fields.gas_limit,
        max_fee_per_gas_wei: fields.max_fee.to_string(),
    })
}

pub(crate) fn sign_approved(
    key: &k256::ecdsa::SigningKey,
    policy: &AdmissionPolicy,
    verified: &VerifiedOperation,
    nonce: u64,
    priority: U256,
    max_fee: U256,
) -> Result<String> {
    let fields = OuterFields {
        chain_id: policy.domain.chain_id,
        nonce,
        priority,
        max_fee,
        gas_limit: policy.gas_limit,
        to: bounded_hex(&policy.domain.pool_contract, 20)?
            .try_into()
            .map_err(|_| Error::InvalidPolicy)?,
        call_data: bounded_hex(&verified.call_data, MAX_CALLDATA)?,
    };
    let hash = Keccak256::digest(fields.encode(None));
    let (signature, recovery) = key.sign_prehash_recoverable(&hash);
    Ok(format!(
        "0x{}",
        hex::encode(fields.encode(Some((&signature, recovery))))
    ))
}

#[cfg(test)]
pub(crate) fn sign_fixture(
    policy: &AdmissionPolicy,
    verified: &VerifiedOperation,
    nonce: u64,
    fee: u64,
) -> String {
    // Deliberately public development key. There is NO runtime signing API.
    let key = k256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
    let fields = OuterFields {
        chain_id: policy.domain.chain_id,
        nonce,
        priority: 1u8.into(),
        max_fee: fee.into(),
        gas_limit: policy.gas_limit,
        to: bounded_hex(&policy.domain.pool_contract, 20)
            .unwrap()
            .try_into()
            .unwrap(),
        call_data: bounded_hex(&verified.call_data, MAX_CALLDATA).unwrap(),
    };
    let hash = Keccak256::digest(fields.encode(None));
    let (signature, recovery) = key.sign_prehash_recoverable(&hash);
    format!(
        "0x{}",
        hex::encode(fields.encode(Some((&signature, recovery))))
    )
}

#[cfg(test)]
pub(crate) fn fixture_address() -> String {
    let key = k256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap();
    let bytes = key.verifying_key().to_sec1_point(false);
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(&bytes.as_bytes()[1..])[12..])
    )
}
