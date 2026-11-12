#!/usr/bin/env node
/**
 * Faz 28 (P1-7 — bağımsız denetim): SQLite yedekleme — WAL-tutarlı.
 *
 * ESKİ DURUM: copyFileSync ile canlı WAL-mod DB'nin ana dosyası kopyalanıyordu
 * — aktif yazım ortasında yırtık/tutarsız snapshot üretebiliyordu (ANA + WAL
 * ayrı kopyalanır; aradaki yazım anlık görüntüde kaybolur).
 *
 * YENİ DURUM: node:sqlite (Node >= 22.5) ile `VACUUM INTO` — motorun
 * işaretlediği tutarlı anlık görüntü, tek dosya, WAL/SHM yok. node:sqlite
 * yoksa (eski Node) davranış eskisi gibi kopya + açık uyarı.
 *
 * Retention: son 7 gün korunur.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");

const dbPath = process.env.DCA_DB_PATH || path.join(rootDir, ".local-runtime", "paper.sqlite3");
const backupDir = process.env.DCA_BACKUP_DIR || path.join(rootDir, "backups");

console.log(`[backup] Starting database backup routine for: ${dbPath}`);

if (!fs.existsSync(dbPath)) {
  console.warn(`[backup] Database file does not exist at ${dbPath}. Skipping backup.`);
  process.exit(0);
}

fs.mkdirSync(backupDir, { recursive: true });

const now = new Date();
const pad = (n) => String(n).padStart(2, "0");
const timestamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
const backupFile = path.join(backupDir, `dcabot_backup_${timestamp}.sqlite3`);

try {
  // 1. Tutarlı anlık görüntü — node:sqlite VACUUM INTO (tutarlı + sıkıştırır).
  let usedVacuum = false;
  try {
    const { DatabaseSync } = await import("node:sqlite");
    const src = new DatabaseSync(dbPath, { readOnly: true });
    // VACUUM INTO: kaynak DB'nin transaction-anlık görüntüsünden bağımsız,
    // kendi kendine yeterli tek dosya üretir (WAL da dahil). Yol SQL string
    // literal'i — tek tırnak, içtekiler kaçırılır.
    const lit = `'${backupFile.replace(/\\/g, "/").replace(/'/g, "''")}'`;
    src.exec(`VACUUM INTO ${lit}`);
    src.close();
    usedVacuum = true;
  } catch (err) {
    // node:sqlite yok / açılamadı (eski Node veya DB kilitli) → eski yolla
    // kopyala; uyarıyı SAHTELEMEK yok — log açıkça "tutarlı değil" der.
    console.warn(`[backup] WARN: VACUUM INTO kullanılamadı (${err.message}) — ham kopya DEVAM EDİYOR; bu kopya canlı yazım anında tutarsız olabilir.`);
    fs.copyFileSync(dbPath, backupFile);
    for (const suffix of ["-wal", "-shm"]) {
      const side = `${dbPath}${suffix}`;
      if (fs.existsSync(side)) fs.copyFileSync(side, `${backupFile}${suffix}`);
    }
  }

  const stat = fs.statSync(backupFile);
  console.log(`[backup] Snapshot created: ${backupFile} (${stat.size} bytes, mode=${usedVacuum ? "vacuum-into (tutarlı)" : "raw-copy (riskli)"})`);

  // 2. Retention: Keep last 7 days of backups
  const maxAgeMs = 7 * 24 * 60 * 60 * 1000;
  const files = fs.readdirSync(backupDir);
  for (const f of files) {
    if (f.startsWith("dcabot_backup_") && f.endsWith(".sqlite3")) {
      const fullPath = path.join(backupDir, f);
      const fStat = fs.statSync(fullPath);
      if (Date.now() - fStat.mtimeMs > maxAgeMs) {
        console.log(`[backup] Pruning expired snapshot: ${f}`);
        fs.unlinkSync(fullPath);
        if (fs.existsSync(`${fullPath}-wal`)) fs.unlinkSync(`${fullPath}-wal`);
        if (fs.existsSync(`${fullPath}-shm`)) fs.unlinkSync(`${fullPath}-shm`);
      }
    }
  }

  console.log("[backup] SUCCESS: Database backup and retention cycle complete.");
} catch (err) {
  console.error(`[backup] ERROR: Backup failed: ${err.message}`);
  process.exit(1);
}
