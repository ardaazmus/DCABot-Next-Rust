# DCABOT v3.0.0 — İNDİRME ve ÇALIŞTIRMA KILAVUZU

Bu paket DCABOT v3.0.0'ın TAM kaynak kodudur (Next.js UI + Rust dca-core motoru).

## Paket İçeriği
- `src/` — Next.js 16 terminal arayüzü (Kokpit, Wizard, Studio, Lab, Fleet, Kill Switch, Settings)
- `mini-services/dca-core/` — Rust workspace (7 crate): dca_math (4096-bit kesir), dca_kernel (8 gate),
  dca_strategy (DCA/Grid/Hedge/Pine/backtest), dca_pine (Pine v5 uyumlu alt küme), dca_exchange (Bybit/Binance/Paper),
  dca_risk, dca_server (REST + SSE + webhook)
- `tests/e2e_engine.py` — 165 assertion'lık API E2E süiti
- `docs/` — ana spesifikasyon, Bybit araştırması, en güncel ARAŞTIRMA RAPORU (2025-10-21)
- `blueprint/` — 3Commas tarzı giriş koşulları + piyasa korumaları spesifikasyonları
- `.env.example` — ortam değişkeni şablonu

## Hızlı Başlangıç

### 1) UI (Next.js 16)
```bash
bun install          # node_modules
cp .env.example .env # gerekirse düzenle
bun run dev          # http://localhost:3000
```

### 2) Motor (Rust dca-core)
```bash
# Rust yoksa: https://rustup.rs (stable ≥ 1.99)
cd mini-services/dca-core
cargo build --release
./run.sh             # veya: target/release/dca_server  (port 3002)
```

### 3) Doğrulama
```bash
curl localhost:3002/api/health
# {"service":"dca-core","status":"ok","version":"3.0.0","gates":8, ...}
python3 tests/e2e_engine.py   # UI+motor ayaktayken: 165/165
cargo test --release          # 195/195
```

## Notlar
- UI → motor bağlantısı gateway üzerinden `?XTransformPort=3002` sözleşmesiyledir (Caddyfile'a bakın).
- `db/` klasörü KULLANICI VERİSİ içerdiğinden bu pakete KOYULMADI — motor ilk açılışta şemayı kendisi kurar.
- PAPER modu varsayılan ve güvenli moddur; LIVE için borsa API anahtarları Settings > Vault'tan girilir.
- Ayrıntılı güncel durum ve dürüst sınırlar: `docs/ARASTIRMA_RAPORU_2025-10-21.md`
