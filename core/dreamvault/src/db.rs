use crate::error::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

pub type Pool = SqlitePool;

/// Open (creating if needed) the SQLite database at `path` and run migrations.
pub async fn connect(path: &Path) -> Result<Pool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let url = format!("sqlite://{}", path.display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true)
        // WAL keeps reads fast while a scan writes — matters for large libraries.
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        // NORMAL is the documented-safe companion to WAL: COMMIT no longer fsyncs
        // (fsync happens only at checkpoint), so a big scan that writes thousands
        // of rows isn't bottlenecked on one disk flush per row. The only durability
        // cost is losing the very last transaction on a power loss — acceptable for
        // a library index that can simply be re-scanned; no corruption risk.
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;

    migrate(&pool).await?;
    Ok(pool)
}

/// Open an in-memory database (used by tests).
pub async fn connect_in_memory() -> Result<Pool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &Pool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
