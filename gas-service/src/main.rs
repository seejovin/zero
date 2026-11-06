use std::io::Read;
use std::path::Path;
use wallet_gas_service::runtime::{OperatorRuntime, RuntimeConfig};
use wallet_gas_service::{capabilities, inspect_database, AdmissionPolicy};

fn read_policy(path: &str) -> Result<AdmissionPolicy, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "Unable to open policy")?;
    let mut text = String::new();
    file.take(1_048_577)
        .read_to_string(&mut text)
        .map_err(|_| "Unable to read policy")?;
    AdmissionPolicy::parse(&text).map_err(|_| "Policy validation failed")
}

fn run() -> Result<(), &'static str> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [command, path] = args.as_slice() {
        if command == "serve" {
            let config = RuntimeConfig::read(Path::new(path))
                .map_err(|_| "Runtime configuration rejected")?;
            return OperatorRuntime::build(config)
                .map_err(|_| "Runtime initialization rejected")?
                .serve()
                .map_err(|_| "Gateway stopped");
        }
    }
    let output = match args.as_slice() {
        [] => serde_json::to_value(capabilities()).map_err(|_| "Unable to encode capabilities")?,
        [command] if command == "capabilities" => serde_json::to_value(capabilities()).map_err(|_| "Unable to encode capabilities")?,
        [command, path] if command == "inspect" => {
            let summary = inspect_database(Path::new(path)).map_err(|_| "Operator journal could not be verified; no records were changed")?;
            serde_json::to_value(summary).map_err(|_| "Unable to encode journal summary")?
        }
        [command, path] if command == "check-policy" => {
            let policy = read_policy(path)?;
            serde_json::json!({
                "policy_valid": true,
                "policy_digest": policy.digest().map_err(|_| "Policy validation failed")?,
                "production_sponsorship_available": false,
                "note": "Configuration validity does not verify a proof system, authorize sponsorship, or enable a network endpoint."
            })
        }
        [command,path] if command=="check-runtime" => {
            RuntimeConfig::read(Path::new(path)).map_err(|_|"Runtime configuration rejected")?;
            serde_json::json!({"configuration_valid":true,"production_sponsorship_available":false})
        }
        [command,path] if command=="gateway-public-key" => {
            let config=RuntimeConfig::read(Path::new(path)).map_err(|_|"Runtime configuration rejected")?;
            config.public_gateway_configuration().map_err(|_|"Public configuration unavailable")?
        }
        [command,policy_path,recipient_path,key_path] if command=="sign-fee-schedule" => {
            let policy=read_policy(policy_path)?;
            let file=std::fs::File::open(recipient_path).map_err(|_|"Unable to read public fee recipient")?;
            let mut recipient=String::new();file.take(258).read_to_string(&mut recipient).map_err(|_|"Unable to read public fee recipient")?;
            if recipient.len()>257 {return Err("Public fee recipient rejected");}
            let recipient=recipient.trim_end_matches(['\r','\n']).to_string();
            let packet=wallet_gas_service::fee_schedule::SignedFeeSchedule::sign_policy(&policy,recipient,&wallet_gas_service::secrets::SecretSource::File(key_path.into())).map_err(|_|"Fee publication rejected")?;
            serde_json::to_value(packet).map_err(|_|"Unable to encode fee publication")?
        }
        _ => return Err("Usage: wallet-gas-service [capabilities | check-policy FILE | check-runtime FILE | inspect DATABASE | gateway-public-key CONFIG | serve CONFIG | sign-fee-schedule POLICY PUBLIC_RECIPIENT_FILE PRIVATE_KEY_FILE]. No arbitrary transaction signing, submission or enable command is available."),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|_| "Unable to encode response")?
    );
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(2);
    }
}
