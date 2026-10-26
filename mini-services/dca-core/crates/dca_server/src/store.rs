//! SQLite WAL persistence layer + AES-256-GCM vault.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use rusqlite::Connection;
use std::sync::Mutex;

pub struct Db {
    pub conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS bots (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                venue TEXT NOT NULL,
                mode TEXT NOT NULL,
                symbol TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'STOPPED',
                config TEXT NOT NULL,
                stats TEXT NOT NULL DEFAULT '{}',
                interval TEXT,
                start_cash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS fills (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                fill_key TEXT NOT NULL,
                order_id TEXT,
                role TEXT,
                side TEXT,
                kind TEXT,
                price TEXT,
                qty TEXT,
                fee TEXT,
                fee_role TEXT,
                realism TEXT,
                ts INTEGER,
                UNIQUE(bot_id, fill_key)
            );
            CREATE INDEX IF NOT EXISTS idx_fills_bot_fillkey ON fills(bot_id, fill_key);
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT,
                level TEXT,
                gate TEXT,
                message TEXT,
                ts INTEGER
            );
            CREATE TABLE IF NOT EXISTS deals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT,
                deal_no INTEGER,
                opened_ts INTEGER,
                closed_ts INTEGER,
                gross_pnl TEXT,
                fees TEXT,
                funding TEXT,
                safety_triggered INTEGER,
                status TEXT
            );
            CREATE TABLE IF NOT EXISTS vault_keys (
                id TEXT PRIMARY KEY,
                venue TEXT NOT NULL,
                label TEXT NOT NULL,
                api_key_enc TEXT NOT NULL,
                api_secret_enc TEXT NOT NULL,
                api_pass_enc TEXT NOT NULL DEFAULT '',
                permissions TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                active INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS equity_points (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                ts INTEGER NOT NULL,
                equity TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS parity_checks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                verdict TEXT NOT NULL,
                count_match TEXT,
                fee_match TEXT,
                bt_fills TEXT,
                paper_fills TEXT,
                win_rate_bt TEXT,
                bars TEXT,
                created_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_events_bot_id ON events(bot_id, id);
            CREATE TABLE IF NOT EXISTS live_orders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                order_id TEXT NOT NULL,
                side TEXT NOT NULL,
                kind TEXT NOT NULL,
                price TEXT NOT NULL,
                qty TEXT NOT NULL,
                role TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'PENDING',
                updated_at INTEGER NOT NULL,
                UNIQUE(bot_id, order_id)
            );
            CREATE INDEX IF NOT EXISTS idx_live_orders_bot ON live_orders(bot_id);
            CREATE TABLE IF NOT EXISTS pair_blacklist (
                pair TEXT PRIMARY KEY,
                reason TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bot_memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_id TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'SUMMARY',
                verdict TEXT NOT NULL,
                content TEXT,
                context TEXT NOT NULL DEFAULT '{}',
                model TEXT NOT NULL DEFAULT '',
                note TEXT,
                elapsed_ms INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS pine_scripts (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                code TEXT NOT NULL,
                inputs TEXT NOT NULL DEFAULT '{}',
                notes TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        Self::migrate_columns(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Additive column migrations for databases created before a column existed.
    /// Guarded by PRAGMA table_info so repeated boots are no-ops.
    fn migrate_columns(conn: &Connection) -> Result<(), String> {
        let cols = |t: &str| -> Result<Vec<String>, String> {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({t})"))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(|e| e.to_string())?;
            Ok(rows.filter_map(|x| x.ok()).collect())
        };
        let existing = cols("bots")?;
        if !existing.iter().any(|c| c == "interval") {
            conn.execute("ALTER TABLE bots ADD COLUMN interval TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        if !existing.iter().any(|c| c == "start_cash") {
            conn.execute("ALTER TABLE bots ADD COLUMN start_cash TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        // P4-3 OKX passphrase: eski kasalara sonradan eklenir (boş = yok).
        let vault_cols = cols("vault_keys")?;
        if !vault_cols.iter().any(|c| c == "api_pass_enc") {
            conn.execute("ALTER TABLE vault_keys ADD COLUMN api_pass_enc TEXT NOT NULL DEFAULT ''", [])
                .map_err(|e| e.to_string())?;
        }
        // Faz 3 (C-4): kalıcı runtime snapshot — motor fazı + stepper durumu.
        if !existing.iter().any(|c| c == "runtime_state") {
            conn.execute("ALTER TABLE bots ADD COLUMN runtime_state TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        // Faz 2.4 adli tıp: tick izi (NULL = göç-öncesi olay / tick-dışı yazım).
        let event_cols = cols("events")?;
        if !event_cols.iter().any(|c| c == "trace_id") {
            conn.execute("ALTER TABLE events ADD COLUMN trace_id TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// ---------------- AES-256-GCM Vault ----------------

pub struct Vault {
    cipher: Aes256Gcm,
}

impl Vault {
    /// Master key: env DCA_VAULT_KEY (base64/hex/plain 32 bytes) REQUIRED.
    /// The master key is NEVER generated or persisted in the DB. If the env
    /// var is missing, vault construction fails (fail-closed) — encrypted
    /// API keys cannot be decrypted without the out-of-DB master key.
    pub fn new(db: &Db) -> Result<Self, String> {
        let key_bytes = {
            let from_env = std::env::var("DCA_VAULT_KEY")
                .map_err(|_| "DCA_VAULT_KEY environment variable is required — master key must be provided out-of-DB (SEC-02)")?;
            derive_32(&from_env)
        };
        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
        Ok(Self { cipher: Aes256Gcm::new(key) })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| "vault encrypt failed".to_string())?;
        let mut out = nonce_bytes.to_vec();
        out.extend(ct);
        Ok(hex::encode(out))
    }

    pub fn decrypt(&self, hexdata: &str) -> Result<String, String> {
        let data = hex::decode(hexdata).map_err(|e| e.to_string())?;
        if data.len() < 13 {
            return Err("vault data too short".into());
        }
        let (nonce_bytes, ct) = data.split_at(12);
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|_| "vault decrypt failed".to_string())?;
        String::from_utf8(pt).map_err(|e| e.to_string())
    }
}

fn derive_32(s: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().to_vec()
}

/// Resolve the newest ACTIVE vault key for a venue into plaintext credentials.
/// Used to arm LIVE-mode bots at creation/restore. Errors surface decryption
/// problems; None means the venue simply has no usable key yet.
/// P4-3: OKX passphrase üçüncü bileşendir (boş kayıt = None).
pub fn load_active_key(db: &Db, vault: &Vault, venue: &str) -> Result<Option<dca_exchange::VenueCreds>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT api_key_enc, api_secret_enc, api_pass_enc FROM vault_keys WHERE venue=?1 AND active=1 ORDER BY created_at DESC LIMIT 1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([venue]).map_err(|e| e.to_string())?;
    match rows.next().map_err(|e| e.to_string())? {
        Some(row) => {
            let ek: String = row.get(0).map_err(|e| e.to_string())?;
            let es: String = row.get(1).map_err(|e| e.to_string())?;
            let ep: String = row.get(2).map_err(|e| e.to_string())?;
            let k = vault.decrypt(&ek)?;
            let s = vault.decrypt(&es)?;
            let p = if ep.trim().is_empty() { None } else { Some(vault.decrypt(&ep)?) };
            Ok(Some(dca_exchange::VenueCreds { key: k, secret: s, passphrase: p }))
        }
        None => Ok(None),
    }
}
