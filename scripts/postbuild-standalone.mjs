#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, "..");

const standaloneDir = path.join(rootDir, ".next", "standalone");
const standaloneNextDir = path.join(standaloneDir, ".next");
const staticSrc = path.join(rootDir, ".next", "static");
const staticDest = path.join(standaloneNextDir, "static");
const publicSrc = path.join(rootDir, "public");
const publicDest = path.join(standaloneDir, "public");

console.log("[postbuild] Finalizing Next.js standalone build distribution...");

try {
  if (!fs.existsSync(standaloneDir)) {
    throw new Error(`Standalone build output directory not found: ${standaloneDir}`);
  }

  // 1. Copy .next/static -> .next/standalone/.next/static
  if (fs.existsSync(staticSrc)) {
    console.log(`[postbuild] Copying static assets from ${staticSrc} to ${staticDest}`);
    fs.mkdirSync(standaloneNextDir, { recursive: true });
    fs.cpSync(staticSrc, staticDest, { recursive: true });
  } else {
    throw new Error(`Static assets directory not found: ${staticSrc}`);
  }

  // 2. Copy public -> .next/standalone/public
  if (fs.existsSync(publicSrc)) {
    console.log(`[postbuild] Copying public folder from ${publicSrc} to ${publicDest}`);
    fs.cpSync(publicSrc, publicDest, { recursive: true });
  }

  // 3. Validation: Verify destination static directory contains files
  const staticFiles = fs.readdirSync(staticDest);
  if (staticFiles.length === 0) {
    throw new Error(`Verification failed: ${staticDest} is empty!`);
  }

  console.log(`[postbuild] SUCCESS: Standalone distribution ready with ${staticFiles.length} static bundles.`);
} catch (err) {
  console.error(`[postbuild] FAILED: ${err.message}`);
  process.exit(1);
}
