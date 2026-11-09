#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");

const dbPath = process.env.DCA_DB_PATH || path.join(rootDir, ".local-runtime", "paper.sqlite3");
const backupDir = path.join(rootDir, "backups");

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
  // 1. Copy database snapshot
  fs.copyFileSync(dbPath, backupFile);
  const walPath = `${dbPath}-wal`;
  if (fs.existsSync(walPath)) {
    fs.copyFileSync(walPath, `${backupFile}-wal`);
  }
  const shmPath = `${dbPath}-shm`;
  if (fs.existsSync(shmPath)) {
    fs.copyFileSync(shmPath, `${backupFile}-shm`);
  }

  const stat = fs.statSync(backupFile);
  console.log(`[backup] Snapshot created: ${backupFile} (${stat.size} bytes)`);

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
