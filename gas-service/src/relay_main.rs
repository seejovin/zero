//! Run this binary under an independently controlled operator and OS identity.
//! It never loads the gas-service RuntimeConfig or any wallet/signing secret.
use std::path::Path;
use wallet_gas_service::relay::{self, RelayConfig};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, path] if command == "check-config" => {
            RelayConfig::read(Path::new(path)).map(|_| {
                println!(
                    "{{\"configuration_valid\":true,\"independent_operator_verified\":false}}"
                );
            })
        }
        [command, path] if command == "serve" => {
            RelayConfig::read(Path::new(path)).and_then(relay::run)
        }
        _ => {
            eprintln!("Usage: wallet-ohttp-relay [check-config CONFIG | serve CONFIG]");
            std::process::exit(2);
        }
    };
    if result.is_err() {
        eprintln!("Relay configuration or transport unavailable");
        std::process::exit(2);
    }
}
