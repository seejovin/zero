//! Explicit operator-secret inputs. Secret values are never command arguments,
//! serialized configuration fields, Debug output or error-message details.
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use crate::{Error, Result};

pub enum SecretSource {
    File(PathBuf),
    Stdin,
    Environment(String),
}

impl SecretSource {
    pub fn read(&self) -> Result<Zeroizing<[u8; 32]>> {
        match self {
            Self::File(path) => read_file(path),
            Self::Stdin => read_hex(&mut std::io::stdin().lock()),
            Self::Environment(name) => {
                if name.is_empty()
                    || name.len() > 128
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                {
                    return Err(Error::SecretInput);
                }
                let value = Zeroizing::new(std::env::var(name).map_err(|_| Error::SecretInput)?);
                parse_hex(value.as_bytes())
            }
        }
    }
}

fn read_file(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let before = std::fs::symlink_metadata(path).map_err(|_| Error::SecretInput)?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err(Error::SecretInput);
    }
    let mut file = std::fs::File::open(path).map_err(|_| Error::SecretInput)?;
    let after = file.metadata().map_err(|_| Error::SecretInput)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.ino() != after.ino()
            || before.dev() != after.dev()
            || after.permissions().mode() & 0o077 != 0
        {
            return Err(Error::SecretInput);
        }
    }
    if after.len() > 68 {
        return Err(Error::SecretInput);
    }
    read_hex(&mut file)
}

fn read_hex(reader: &mut impl Read) -> Result<Zeroizing<[u8; 32]>> {
    let mut input = Zeroizing::new(Vec::new());
    reader
        .take(69)
        .read_to_end(&mut input)
        .map_err(|_| Error::SecretInput)?;
    parse_hex(&input)
}

fn parse_hex(input: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    let input = input.strip_suffix(b"\r").unwrap_or(input);
    let input = input.strip_prefix(b"0x").unwrap_or(input);
    if input.len() != 64 || !input.iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::SecretInput);
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    hex::decode_to_slice(input, secret.as_mut()).map_err(|_| Error::SecretInput)?;
    if secret.iter().all(|b| *b == 0) {
        return Err(Error::SecretInput);
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_file_is_bounded_and_rejects_symlinks_or_public_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("operator-key");
        std::fs::write(&path, hex::encode([7u8; 32])).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(SecretSource::File(path.clone()).read().is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let link = temp.path().join("link");
            symlink(&path, &link).unwrap();
            assert!(SecretSource::File(link).read().is_err());
        }
        assert_eq!(*SecretSource::File(path.clone()).read().unwrap(), [7u8; 32]);
        std::fs::write(&path, "aa".repeat(100)).unwrap();
        assert!(SecretSource::File(path).read().is_err());
        assert!(parse_hex(&[b'0'; 64]).is_err());
    }
}
