#!/bin/bash
# Portable Next.js dev runner
set -e
ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"
exec npm run dev
