//! Protocol-specific derivation for the isolated local note engine.
//! The BIP39 seed and public-wallet signing key never cross this boundary.
//! Paths match the pinned RAILGUN engine; no derivation or signing is delegated
//! to an RPC. Returned bytes are secret and must only enter the offline engine.
use crate::{crypto, WalletError};
use hmac::{Hmac, KeyInit, Mac};
use sha2_modern::{Sha256, Sha512};
use zeroize::Zeroizing;

fn hmac512(key: &[u8], data: &[u8]) -> Result<Zeroizing<[u8; 64]>, WalletError> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key).map_err(|_| WalletError::Crypto)?;
    mac.update(data);
    Ok(Zeroizing::new(mac.finalize().into_bytes().into()))
}
fn derive(seed: &[u8], purpose: u32) -> Result<Zeroizing<[u8; 32]>, WalletError> {
    let mut node = hmac512(b"babyjubjub seed", seed)?;
    for index in [purpose, 1984, 0, 0, 0] {
        let mut input = Zeroizing::new([0u8; 37]);
        input[1..33].copy_from_slice(&node[..32]);
        input[33..].copy_from_slice(&(index | 0x8000_0000).to_be_bytes());
        node = hmac512(&node[32..], &input[..])?;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&node[..32]);
    Ok(key)
}

/// Layout: spending private key (32), viewing private key (32), independent
/// encrypted engine-cache key (32). Index zero is the deliberately fixed
/// candidate account; this does not change existing EVM derivation.
#[cfg_attr(feature = "native", uniffi::export)]
pub fn private_engine_keys(
    vault_hex: String,
    vault_key: Vec<u8>,
    wallet_identity: String,
) -> Result<Vec<u8>, WalletError> {
    crate::private_payment::canonical_hash(&wallet_identity)?;
    let key = Zeroizing::new(vault_key);
    let seed = crypto::open_seed(&vault_hex, &key)?;
    let spending = derive(&seed[..], 44)?;
    let viewing = derive(&seed[..], 420)?;
    let mut state_key = Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<Sha256>::new(Some(b"local-wallet/private-engine-cache/v1"), &seed[..])
        .expand(wallet_identity.as_bytes(), &mut state_key[..])
        .map_err(|_| WalletError::Crypto)?;
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&spending[..]);
    out.extend_from_slice(&viewing[..]);
    out.extend_from_slice(&state_key[..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_reference_engine_derivation() {
        // Public BIP39 known-answer fixture; never use this mnemonic for funds.
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let seed = bip39::Mnemonic::parse(phrase).unwrap().to_seed("");
        assert_eq!(
            hex::encode(&derive(&seed, 44).unwrap()[..]),
            "08b2d974aa7fffd9d068b78c34434c534ddcd9343fcbf5aa12cf78e1a3c1ccb9"
        );
        assert_eq!(
            hex::encode(&derive(&seed, 420).unwrap()[..]),
            "9a9e1ca3b9476dc8500b43f30f34104c92a3eedfd727757ffd0ad15da8e11572"
        );
    }
    #[test]
    fn domains_and_identity_are_separated() {
        let seed = [7u8; 64];
        assert_ne!(*derive(&seed, 44).unwrap(), *derive(&seed, 420).unwrap());
        let hk = hkdf::Hkdf::<Sha256>::new(Some(b"local-wallet/private-engine-cache/v1"), &seed);
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        hk.expand(b"a", &mut a).unwrap();
        hk.expand(b"b", &mut b).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, *derive(&seed, 44).unwrap());
    }
}
