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
 * yoksa yedekleme açıkça başarısız olur; tutarsız ham kopya üretilmez.
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
const timestamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}${String(now.getMilliseconds()).padStart(3, "0")}`;
const backupFile = path.join(backupDir, `dcabot_backup_${timestamp}_${process.pid}.sqlite3`);
if (fs.existsSync(backupFile)) {
  throw new Error(`Backup target already exists; refusing to overwrite it: ${backupFile}`);
}

try {
  // 1. Tutarlı anlık görüntü — yalnız node:sqlite VACUUM INTO kabul edilir.
  // Ham DB/WAL kopyası WAL checkpoint anında bile atomik değildir; sürüm
  // desteklemiyorsa yanıltıcı bir yedek üretmek yerine görev başarısız olmalı.
  let DatabaseSync;
  try {
    ({ DatabaseSync } = await import("node:sqlite"));
  } catch (err) {
    throw new Error(`Tutarlı yedek için Node.js 22.5+ node:sqlite gerekir (${err.message})`);
  }

  const src = new DatabaseSync(dbPath, { readOnly: true });
  try {
    // VACUUM INTO: kaynak DB'nin transaction-anlık görüntüsünden bağımsız,
    // kendi kendine yeterli tek dosya üretir (WAL da dahil). Yol SQL string
    // literal'i — tek tırnak, içtekiler kaçırılır.
    const lit = `'${backupFile.replace(/\\/g, "/").replace(/'/g, "''")}'`;
    src.exec(`VACUUM INTO ${lit}`);
  } finally {
    src.close();
  }

  // Yedeği retention listesine almadan önce SQLite bütünlüğünü doğrula.
  const check = new DatabaseSync(backupFile, { readOnly: true });
  let integrity;
  try {
    integrity = check.prepare("PRAGMA quick_check").get()?.quick_check;
  } finally {
    check.close();
  }
  if (integrity !== "ok") throw new Error(`Yedek bütünlük denetimi başarısız: ${String(integrity)}`);

  const stat = fs.statSync(backupFile);
  console.log(`[backup] Snapshot created: ${backupFile} (${stat.size} bytes, mode=vacuum-into, integrity=ok)`);

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
  if (fs.existsSync(backupFile)) fs.unlinkSync(backupFile);
  console.error(`[backup] ERROR: Backup failed: ${err.message}`);
  process.exit(1);
}
