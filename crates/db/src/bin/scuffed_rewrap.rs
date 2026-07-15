//! CLI: re-encrypt all EncryptedBlob fields under the current ENCRYPTION_KEY.
//!
//! Usage:
//!   ENCRYPTION_KEY=... ENCRYPTION_KEY_VERSION=2 ENCRYPTION_KEY_PREVIOUS=1:... \
//!   SURREALDB_URL=... PRODUCTION=1 scuffed-rewrap
//!
//! Dev / in-memory (also needs ENCRYPTION_KEY for the crypto service):
//!   ENCRYPTION_KEY=... scuffed-rewrap --memory
//!
//! Exit codes: 0 on success (including zero rows), 1 on config/crypto/db error.

use scuffed_db::migrations::run_migrations;
use scuffed_db::{rewrap_all_encrypted_fields, Database};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("scuffed-rewrap error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let memory = std::env::args().any(|a| a == "--memory" || a == "-m");

    let db = if memory {
        eprintln!("connecting to in-memory SurrealDB (dev)");
        let db = Database::connect_memory().await?;
        // connect_memory does not migrate; schema required for SELECT/UPDATE.
        run_migrations(&db.client).await?;
        db
    } else {
        eprintln!("connecting via Database::connect_from_env");
        // connect_from_env runs migrations for remote/scoped bootstraps.
        Database::connect_from_env().await?
    };

    if db.crypto.is_none() {
        return Err("ENCRYPTION_KEY is not set — cannot rewrap without a CryptoService".into());
    }

    let version = db.crypto.as_ref().unwrap().key_version();
    eprintln!("current key version: {version}");
    eprintln!("scanning encrypted fields…");

    let stats = rewrap_all_encrypted_fields(&db).await?;
    println!("{stats}");
    eprintln!(
        "done: rewrapped {} of {} scanned ({} already current)",
        stats.rewrapped, stats.scanned, stats.skipped_current
    );
    Ok(())
}
