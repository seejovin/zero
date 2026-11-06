//! Fixed, hash-pinned offline protocol verifier subprocess. No shell, request-
//! selected program, request key material, inherited environment or stderr logs.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wallet_core::private_payment::{validate_envelope, PrivateEnvelope};
use zeroize::Zeroizing;

use crate::model::{decimal, decode, encode, fixed_hex};
use crate::{Error, ProofVerifier, Result, VerifiedOperation};

const MAX_OUTPUT: usize = 300_000;
const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVerifierConfig {
    pub node_executable: PathBuf,
    pub node_sha256: String,
    pub script: PathBuf,
    pub script_sha256: String,
    pub operator_config: PathBuf,
    pub operator_config_sha256: String,
    pub additional_native_fee_cap_wei: String,
}

impl NodeVerifierConfig {
    pub fn validate_shape(&self) -> Result<()> {
        decimal(&self.additional_native_fee_cap_wei)?;
        for path in [&self.node_executable, &self.script, &self.operator_config] {
            if !path.is_absolute() {
                return Err(Error::InvalidPolicy);
            }
        }
        for digest in [
            &self.node_sha256,
            &self.script_sha256,
            &self.operator_config_sha256,
        ] {
            fixed_hex(digest, 32)?;
        }
        Ok(())
    }
}

pub struct NodeProofVerifier {
    config: NodeVerifierConfig,
    in_flight: AtomicUsize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema_version: u8,
    operation_digest: String,
    tree_number: u64,
    call_data: String,
    fee_output_commitment: String,
    additional_native_fee_cap_wei: String,
}

impl NodeProofVerifier {
    pub fn new(config: NodeVerifierConfig) -> Result<Self> {
        config.validate_shape()?;
        check_file(
            &config.node_executable,
            &config.node_sha256,
            128 * 1024 * 1024,
        )?;
        check_file(&config.script, &config.script_sha256, 2 * 1024 * 1024)?;
        check_file(
            &config.operator_config,
            &config.operator_config_sha256,
            128 * 1024,
        )?;
        check_private_config(&config.operator_config)?;
        Ok(Self {
            config,
            in_flight: AtomicUsize::new(0),
        })
    }

    async fn execute(&self, input: &[u8]) -> Result<Vec<u8>> {
        // Fixed argv, empty inherited environment. Keys are read by the reviewed
        // verifier from its fixed operator configuration, not supplied on stdin.
        let mut command = tokio::process::Command::new(&self.config.node_executable);
        command
            .arg(&self.config.script)
            .arg("--config")
            .arg(&self.config.operator_config)
            .env_clear()
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| Error::VerificationFailed)?;
        let mut stdin = child.stdin.take().ok_or(Error::VerificationFailed)?;
        let stdout = child.stdout.take().ok_or(Error::VerificationFailed)?;
        let operation = async {
            let write = async move {
                stdin
                    .write_all(input)
                    .await
                    .map_err(|_| Error::VerificationFailed)?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|_| Error::VerificationFailed)?;
                // Pipes need the handle closed to deliver EOF. Keeping stdin
                // alive until try_join completes deadlocks a verifier that reads
                // the complete bounded request before producing its response.
                drop(stdin);
                Ok(())
            };
            let read = async {
                let mut output = Vec::new();
                stdout
                    .take((MAX_OUTPUT + 1) as u64)
                    .read_to_end(&mut output)
                    .await
                    .map_err(|_| Error::VerificationFailed)?;
                if output.is_empty() || output.len() > MAX_OUTPUT {
                    return Err(Error::VerificationFailed);
                }
                Ok(output)
            };
            let wait = async { child.wait().await.map_err(|_| Error::VerificationFailed) };
            let (_, output, status) = tokio::try_join!(write, read, wait)?;
            if !status.success() {
                return Err(Error::VerificationFailed);
            }
            Ok(output)
        };
        match tokio::time::timeout(DEADLINE, operation).await {
            Ok(Ok(output)) => Ok(output),
            _ => {
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                Err(Error::VerificationFailed)
            }
        }
    }
}

impl ProofVerifier for NodeProofVerifier {
    fn verify(&self, envelope: &PrivateEnvelope) -> Result<VerifiedOperation> {
        validate_envelope(envelope).map_err(|_| Error::VerificationFailed)?;
        self.in_flight
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| Error::BudgetExceeded)?;
        struct Permit<'a>(&'a AtomicUsize);
        impl Drop for Permit<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Release);
            }
        }
        let _permit = Permit(&self.in_flight);
        // Artifact replacement is rejected before every execution. Dependency
        // tree qualification and OS sandboxing remain deployment requirements.
        check_file(
            &self.config.node_executable,
            &self.config.node_sha256,
            128 * 1024 * 1024,
        )?;
        check_file(
            &self.config.script,
            &self.config.script_sha256,
            2 * 1024 * 1024,
        )?;
        check_file(
            &self.config.operator_config,
            &self.config.operator_config_sha256,
            128 * 1024,
        )?;
        check_private_config(&self.config.operator_config)?;
        let input = Zeroizing::new(encode(
            &serde_json::json!({"schema_version":1,"envelope":envelope}),
        )?);
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Error::VerificationFailed);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Error::VerificationFailed)?;
        let output = runtime.block_on(self.execute(input.as_bytes()))?;
        let text = std::str::from_utf8(&output).map_err(|_| Error::VerificationFailed)?;
        let reply: Reply = decode(text).map_err(|_| Error::VerificationFailed)?;
        if reply.schema_version != 1
            || reply.operation_digest != envelope.operation_digest
            || reply.call_data != envelope.call_data
            || reply.tree_number > 65_535
            || reply.tree_number != u64::from(envelope.tree_number)
            || reply.fee_output_commitment != envelope.fee.fee_output_commitment
            || reply.additional_native_fee_cap_wei != self.config.additional_native_fee_cap_wei
        {
            return Err(Error::VerificationFailed);
        }
        Ok(VerifiedOperation {
            operation_digest: reply.operation_digest,
            tree_number: reply.tree_number,
            call_data: reply.call_data,
            fee_output_commitment: reply.fee_output_commitment,
            additional_native_fee_cap_wei: reply.additional_native_fee_cap_wei,
        })
    }
}

fn check_private_config(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|_| Error::VerificationFailed)?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(Error::VerificationFailed);
        }
    }
    Ok(())
}

fn check_file(path: &Path, expected: &str, limit: u64) -> Result<()> {
    use std::io::Read;
    fixed_hex(expected, 32)?;
    if !path.is_absolute() {
        return Err(Error::InvalidPolicy);
    }
    let before = std::fs::symlink_metadata(path).map_err(|_| Error::VerificationFailed)?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > limit {
        return Err(Error::VerificationFailed);
    }
    let mut file = std::fs::File::open(path).map_err(|_| Error::VerificationFailed)?;
    let after = file.metadata().map_err(|_| Error::VerificationFailed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.ino() != after.ino()
            || before.dev() != after.dev()
            || after.permissions().mode() & 0o022 != 0
        {
            return Err(Error::VerificationFailed);
        }
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    let mut read = 0u64;
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|_| Error::VerificationFailed)?;
        if length == 0 {
            break;
        }
        read = read
            .checked_add(length as u64)
            .ok_or(Error::VerificationFailed)?;
        if read > limit {
            return Err(Error::VerificationFailed);
        }
        hasher.update(&buffer[..length]);
    }
    if format!("0x{}", hex::encode(hasher.finalize())) != expected {
        return Err(Error::VerificationFailed);
    }
    Ok(())
}
