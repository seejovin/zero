//! Local-only runtime tests. All keys and proofs in this module are public,
//! synthetic fixtures. No public network or funded transaction is involved.
use super::*;
use crate::fee_schedule::{SharedFeeSchedule, SignedFeeSchedule};
use crate::gateway::{AdvisoryStatus, ObliviousGateway, SubmissionResponse};
use crate::signer::{GasQuote, NativeTreasurySigner};
use crate::tests::{envelope, hash, policy, verified, FixtureChain, FixtureVerifier, FEE_ADDRESS};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::mpsc};
use wallet_core::private_payment::envelope_digest;
use wallet_privacy_transport::{GatewayConfig, Operation, OHTTP_RESPONSE_CONTENT_TYPE};
use zeroize::Zeroizing;

pub(crate) fn signed_schedule(p: &AdmissionPolicy) -> (SignedFeeSchedule, [u8; 32]) {
    let key = SigningKey::from_bytes(&[9; 32]);
    let public = key.verifying_key().to_bytes();
    let schedule = SharedFeeSchedule::from_policy(p, FEE_ADDRESS.into()).unwrap();
    let signature = format!(
        "0x{}",
        hex::encode(key.sign(&schedule.signing_bytes().unwrap()).to_bytes())
    );
    (
        SignedFeeSchedule {
            schema_version: 1,
            schedule,
            key_id: format!("0x{}", hex::encode(Sha256::digest(public))),
            signature,
        },
        public,
    )
}

fn gateway(service: Arc<GasService>, p: &AdmissionPolicy) -> Arc<ObliviousGateway> {
    let (packet, key) = signed_schedule(p);
    Arc::new(
        ObliviousGateway::from_seed(
            "gas.example.org".into(),
            1,
            Zeroizing::new([5; 32]),
            service,
            packet,
            key,
        )
        .unwrap(),
    )
}

fn client(gateway: &ObliviousGateway) -> GatewayConfig {
    let key = gateway.public_configuration().unwrap();
    let pin = Sha256::digest(&key).into();
    GatewayConfig::from_pinned_key("gas.example.org".into(), key, pin).unwrap()
}

#[test]
fn shared_fee_packet_interoperates_with_native_wallet_verifier() {
    let (packet, key) = signed_schedule(&policy());
    packet.verify(&key).unwrap();
    let canonical = wallet_core::private_fee_schedule::private_verify_fee_schedule(
        encode(&packet).unwrap(),
        key.to_vec(),
        1500,
    )
    .unwrap();
    assert_eq!(canonical, encode(&packet.schedule).unwrap());
    let mut changed = packet.clone();
    changed.schedule.fee_units = "20001".into();
    assert_eq!(changed.verify(&key), Err(Error::VerificationFailed));
    assert!(
        wallet_core::private_fee_schedule::private_verify_fee_schedule(
            encode(&changed).unwrap(),
            key.to_vec(),
            1500
        )
        .is_err()
    );
    changed = packet.clone();
    changed.schedule.fee_recipient_address = "0zk1differentfeeaddress".into();
    assert!(changed.verify(&key).is_err());
    assert!(
        wallet_core::private_fee_schedule::private_verify_fee_schedule(
            encode(&packet).unwrap(),
            key.to_vec(),
            2000
        )
        .is_err()
    );
}

#[test]
fn daily_shared_schedule_retains_exact_exclusive_expiry_across_service_and_wallet() {
    let mut p = policy();
    p.valid_until = p.valid_from + 86_400;
    let (packet, key) = signed_schedule(&p);
    packet.verify_policy(&key, &p).unwrap();
    let mut e = envelope(11);
    e.fee.quote_valid_until = p.valid_until;
    e.operation_digest = envelope_digest(&e).unwrap();
    for now in [p.valid_from, p.valid_until - 1] {
        p.matches(&e, now).unwrap();
        let canonical = wallet_core::private_fee_schedule::private_verify_fee_schedule(
            encode(&packet).unwrap(),
            key.to_vec(),
            now,
        )
        .unwrap();
        assert_eq!(canonical, encode(&packet.schedule).unwrap());
    }
    for now in [p.valid_from - 1, p.valid_until, p.valid_until + 1, u64::MAX] {
        assert!(p.matches(&e, now).is_err());
        assert!(
            wallet_core::private_fee_schedule::private_verify_fee_schedule(
                encode(&packet).unwrap(),
                key.to_vec(),
                now,
            )
            .is_err()
        );
    }
    p.valid_until += 1;
    assert!(p.validate().is_err());
    let mut oversized = packet;
    oversized.schedule.valid_until += 1;
    assert!(oversized.schedule.signing_bytes().is_err());
    assert!(oversized.verify(&key).is_err());
}

#[test]
fn signed_schedule_rotation_preserves_jobs_claims_and_operator_limits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("journal");
    let service = crate::tests::funded(&path);
    let p = policy();
    let (initial, key) = signed_schedule(&p);
    service.install_fee_schedule(&initial, key).unwrap();
    service
        .admit(&hash(11), &encode(&envelope(11)).unwrap(), 1500)
        .unwrap();
    let mut next = p.clone();
    next.schedule_id = hash(77);
    next.fee_units = "25000".into();
    next.valid_from = 2000;
    next.valid_until = 3000;
    let (packet, _) = signed_schedule(&next);
    service.install_fee_schedule(&packet, key).unwrap();
    let mut env = envelope(12);
    env.fee.schedule_id = next.schedule_id.clone();
    env.fee.fee_units = next.fee_units.clone();
    env.fee.quote_valid_until = next.valid_until;
    env.operation_digest = envelope_digest(&env).unwrap();
    service
        .admit(&hash(12), &encode(&env).unwrap(), 2500)
        .unwrap();
    assert_eq!(service.summary().unwrap().reserved_native_wei, "2000000");
    assert_eq!(
        service
            .admit(&hash(11), &encode(&envelope(11)).unwrap(), 2500)
            .unwrap()
            .phase,
        JobPhase::Admitted
    );
    let mut overlap = next.clone();
    overlap.schedule_id = hash(78);
    overlap.valid_from = 2500;
    overlap.valid_until = 3500;
    assert_eq!(
        service.install_fee_schedule(&signed_schedule(&overlap).0, key),
        Err(Error::PolicyMismatch)
    );
    let mut increased = next;
    increased.gas_limit += 1;
    assert_eq!(
        service.install_fee_schedule(&signed_schedule(&increased).0, key),
        Err(Error::PolicyMismatch)
    );
    assert!(service.install_fee_schedule(&packet, [0; 32]).is_err());
    drop(service);
    let reopened = crate::tests::service(&path);
    assert_eq!(
        reopened.get_job(&hash(12)).unwrap().operation_digest,
        env.operation_digest
    );
    assert_eq!(reopened.summary().unwrap().jobs, 2);
}

#[test]
fn tree_scoped_nullifiers_do_not_conflict_across_different_trees() {
    let temp = tempfile::tempdir().unwrap();
    let service = crate::tests::funded(&temp.path().join("j"));
    let a = envelope(11);
    let mut b = envelope(12);
    b.tree_number = 1;
    b.nullifiers = a.nullifiers.clone();
    b.operation_digest = envelope_digest(&b).unwrap();
    service
        .admit(&hash(11), &encode(&a).unwrap(), 1500)
        .unwrap();
    service
        .admit(&hash(12), &encode(&b).unwrap(), 1500)
        .unwrap();
    assert_eq!(service.summary().unwrap().jobs, 2);
    let mut wrong = verified(&b);
    wrong.tree_number = 0;
    assert_eq!(
        wrong.validate(&b, &policy()),
        Err(Error::VerificationFailed)
    );
}

#[test]
fn native_signer_only_reconstructs_admitted_zero_value_pool_call() {
    let temp = tempfile::tempdir().unwrap();
    let service = crate::tests::funded(&temp.path().join("j"));
    let signer = NativeTreasurySigner::from_key(Zeroizing::new([7; 32])).unwrap();
    let env = envelope(11);
    service
        .admit(&hash(11), &encode(&env).unwrap(), 1500)
        .unwrap();
    let quote = GasQuote {
        nonce: 0,
        max_priority_fee_per_gas_wei: "1".into(),
        max_fee_per_gas_wei: "2".into(),
    };
    let view = service.sign_admitted(&signer, &hash(11), &quote).unwrap();
    assert_eq!(view.phase, JobPhase::Signed);
    let ticket = service.release_for_broadcast(&hash(11)).unwrap();
    let parsed =
        crate::transaction::inspect_outer(&ticket.raw_transaction, &policy(), &verified(&env))
            .unwrap();
    assert_eq!(parsed.transaction_hash, ticket.transaction_hash);
    let other = NativeTreasurySigner::from_key(Zeroizing::new([8; 32])).unwrap();
    let second = envelope(12);
    service
        .admit(&hash(12), &encode(&second).unwrap(), 1500)
        .unwrap();
    assert_eq!(
        service.sign_admitted(&other, &hash(12), &quote),
        Err(Error::InvalidOuterTransaction)
    );
    assert_eq!(
        service.get_job(&hash(12)).unwrap().phase,
        JobPhase::Admitted
    );
}

#[test]
fn real_ohttp_gateway_enforces_private_schema_and_durable_idempotence() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(crate::tests::funded(&temp.path().join("j")));
    let gateway = gateway(service.clone(), &policy());
    let client = client(&gateway);
    let env = envelope(11);
    let body = encode(&env).unwrap();
    for _ in 0..2 {
        let request = client.seal(Operation::Submit, body.as_bytes()).unwrap();
        let response = gateway
            .handle_ciphertext(request.ciphertext(), 1500)
            .unwrap();
        assert!(!response
            .windows(env.operation_digest.len())
            .any(|window| window == env.operation_digest.as_bytes()));
        let opened = request
            .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response)
            .unwrap();
        assert_eq!(opened.status, 202);
        let accepted: SubmissionResponse = serde_json::from_slice(&opened.body).unwrap();
        assert!(accepted.accepted);
        assert_eq!(accepted.operation_digest, env.operation_digest);
    }
    assert_eq!(service.summary().unwrap().jobs, 1);
    let status =
        serde_json::json!({"schema_version":1,"operation_digest":env.operation_digest}).to_string();
    let request = client.seal(Operation::Status, status.as_bytes()).unwrap();
    let response = gateway
        .handle_ciphertext(request.ciphertext(), 1500)
        .unwrap();
    let opened = request
        .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response)
        .unwrap();
    let status: AdvisoryStatus = serde_json::from_slice(&opened.body).unwrap();
    assert_eq!(status.state, "admitted");
    assert_eq!(status.transaction_hash, None);
    let mut leaked = serde_json::to_value(&env).unwrap();
    leaked["recipient"] = "private-recipient-must-not-be-accepted".into();
    let request = client
        .seal(Operation::Submit, leaked.to_string().as_bytes())
        .unwrap();
    let response = gateway
        .handle_ciphertext(request.ciphertext(), 1500)
        .unwrap();
    let opened = request
        .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response)
        .unwrap();
    assert_eq!(opened.status, 422);
    assert!(!String::from_utf8(opened.body)
        .unwrap()
        .contains("recipient"));
    let request = client.seal(Operation::Submit, body.as_bytes()).unwrap();
    let mut altered = request.ciphertext().to_vec();
    *altered.last_mut().unwrap() ^= 1;
    assert!(gateway.handle_ciphertext(&altered, 1500).is_err());
    assert!(gateway.handle_ciphertext(body.as_bytes(), 1500).is_err());
    assert_eq!(service.summary().unwrap().jobs, 1);
}

struct RunningGateway {
    address: SocketAddr,
    stop: tokio::sync::watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for RunningGateway {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn serve(gateway: Arc<ObliviousGateway>) -> RunningGateway {
    let (sender, receiver) = mpsc::channel();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            sender.send(listener.local_addr().unwrap()).unwrap();
            gateway.serve(listener, shutdown).await.unwrap();
        });
    });
    RunningGateway {
        address: receiver.recv().unwrap(),
        stop,
        thread: Some(thread),
    }
}

#[test]
fn loopback_http_accepts_only_ohttp_and_normal_build_stays_closed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(
        GasService::open(
            &temp.path().join("j"),
            policy(),
            Arc::new(FixtureVerifier),
            Arc::new(FixtureChain),
        )
        .unwrap(),
    );
    let gateway = gateway(service.clone(), &policy());
    let private_client = client(&gateway);
    let server = serve(gateway);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap();
    let url = format!("http://{}/ohttp", server.address);
    let env = envelope(11);
    let body = encode(&env).unwrap();
    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let response = client
        .post(format!("http://{}/v1/private/submit", server.address))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
    let request = private_client
        .seal(Operation::Submit, body.as_bytes())
        .unwrap();
    let response = client
        .post(&url)
        .header("content-type", "message/ohttp-req")
        .body(request.ciphertext().to_vec())
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let ciphertext = response.bytes().unwrap();
    let opened = request
        .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &ciphertext)
        .unwrap();
    assert_eq!(opened.status, 503);
    let accepted: SubmissionResponse = serde_json::from_slice(&opened.body).unwrap();
    assert!(!accepted.accepted);
    assert_eq!(service.summary().unwrap().jobs, 0);
    let request = private_client
        .seal(Operation::Submit, body.as_bytes())
        .unwrap();
    let response = client
        .post(&url)
        .header("content-type", "message/ohttp-req")
        .header("cookie", "identity=forbidden")
        .body(request.ciphertext().to_vec())
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    // Send only headers: the server must reject the declared size before it
    // reads a body. Uploading concurrently can correctly receive a BrokenPipe.
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    write!(stream,"POST /ohttp HTTP/1.1\r\nHost: localhost\r\nContent-Type: message/ohttp-req\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",crate::gateway::MAX_ENVELOPE_BYTES+1).unwrap();
    let mut rejected = String::new();
    stream.read_to_string(&mut rejected).unwrap();
    assert!(rejected.starts_with("HTTP/1.1 413"));
}

#[test]
fn public_key_export_is_read_only_and_fee_publisher_never_serializes_private_key() {
    use crate::runtime::{RuntimeConfig, SecretReference};
    let temp = tempfile::tempdir().unwrap();
    let secret = temp.path().join("public-test-seed");
    std::fs::write(&secret, hex::encode([9; 32])).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let p = policy();
    let packet = SignedFeeSchedule::sign_policy(
        &p,
        FEE_ADDRESS.into(),
        &crate::secrets::SecretSource::File(secret.clone()),
    )
    .unwrap();
    let key = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
    assert!(!encode(&packet).unwrap().contains(&hex::encode([9; 32])));
    let config = RuntimeConfig {
        schema_version: 1,
        listen: "127.0.0.1:0".parse().unwrap(),
        authority: "gas.example.org".into(),
        gateway_key_id: 1,
        gateway_secret: SecretReference::File { path: secret },
        journal: temp.path().join("must-not-be-created.sqlite"),
        policy: p,
        fee_schedule: packet,
        pinned_fee_public_key: format!("0x{}", hex::encode(key)),
        proof_verifier: None,
        rpc: None,
        treasury_secret: None,
    };
    let public = config.public_gateway_configuration().unwrap();
    assert!(public["encoded_key"].as_str().unwrap().starts_with("0x01"));
    assert!(!config.journal.exists());
    assert_eq!(public, config.public_gateway_configuration().unwrap());
    let runtime = crate::runtime::OperatorRuntime::build(config.clone()).unwrap();
    assert_eq!(runtime.public_configuration().unwrap(), public);
    assert!(config.journal.exists());
}

#[test]
fn opaque_relay_forwards_genuine_ohttp_but_strips_identity_headers_and_enforces_cors() {
    use crate::relay::{ObliviousRelay, RelayConfig};
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(
        GasService::open(
            &temp.path().join("j"),
            policy(),
            Arc::new(FixtureVerifier),
            Arc::new(FixtureChain),
        )
        .unwrap(),
    );
    let gateway = gateway(service.clone(), &policy());
    let private_client = client(&gateway);
    let gateway_server = serve(gateway);
    let config = RelayConfig {
        schema_version: 1,
        listen: "127.0.0.1:0".parse().unwrap(),
        gateway_url: format!("http://{}/ohttp", gateway_server.address),
        cors_origin: Some("https://wallet.example.org".into()),
    };
    let relay = Arc::new(ObliviousRelay::local_fixture(&config).unwrap());
    let (sender, receiver) = mpsc::channel();
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let thread = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender.send(listener.local_addr().unwrap()).unwrap();
                relay.serve(listener, shutdown).await.unwrap();
            })
    });
    let relay_server = RunningGateway {
        address: receiver.recv().unwrap(),
        stop,
        thread: Some(thread),
    };
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("http://{}/ohttp", relay_server.address);
    let body = encode(&envelope(11)).unwrap();
    let request = private_client
        .seal(Operation::Submit, body.as_bytes())
        .unwrap();
    let response = http
        .post(&url)
        .header("content-type", "message/ohttp-req")
        .header("user-agent", "IDENTITY-MUST-NOT-FORWARD")
        .header("cookie", "user=private")
        .header("authorization", "Bearer private")
        .header("forwarded", "for=192.0.2.1")
        .header("x-forwarded-for", "192.0.2.1")
        .header("origin", "https://wallet.example.org")
        .body(request.ciphertext().to_vec())
        .send()
        .unwrap();
    // The strict downstream gateway rejects any of these original headers. Its
    // genuine authenticated encrypted response proves the relay stripped them.
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "https://wallet.example.org"
    );
    assert!(response
        .headers()
        .get("access-control-allow-credentials")
        .is_none());
    let opened = request
        .open_response(200, OHTTP_RESPONSE_CONTENT_TYPE, &response.bytes().unwrap())
        .unwrap();
    assert_eq!(opened.status, 503);
    assert_eq!(service.summary().unwrap().jobs, 0);
    let preflight = http
        .request(reqwest::Method::OPTIONS, &url)
        .header("origin", "https://wallet.example.org")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type, accept")
        .send()
        .unwrap();
    assert_eq!(preflight.status().as_u16(), 204);
    let bad_origin = http
        .post(&url)
        .header("origin", "https://other.example.org")
        .header("content-type", "message/ohttp-req")
        .body(vec![1; 32])
        .send()
        .unwrap();
    assert_eq!(bad_origin.status().as_u16(), 403);
    assert!(bad_origin
        .headers()
        .get("access-control-allow-origin")
        .is_none());
    let plaintext = http
        .post(&url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .unwrap();
    assert_eq!(plaintext.status().as_u16(), 400);
}
