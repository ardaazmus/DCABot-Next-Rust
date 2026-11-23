# DCABOT v3.0 — Rust Kuantitatif Çekirdek + Next.js Trading Terminali

## Windows üzerinde yerel çalıştırma

Node.js, Rust/MSVC derleme araçları ve proje bağımlılıkları kurulu olmalıdır. Rust paketleri yerel Cargo önbelleğinde bulunmalıdır; başlatıcı kilit dosyasını kullanarak çevrimdışı derler.

```powershell
npm run local:start
```

Arayüz: http://127.0.0.1:3000. Başlatıcı önce güncel Rust motorunu derler, ardından motoru ve Next.js'i gizli arka plan süreçlerinde başlatır; iki servisin HTTP yanıtını kontrol eder. Derleme paralelliği ve SWC iş parçacığı sayısı sınırlıdır. 3000/3002 portları boş olmalıdır.

**Kimlik doğrulama (SEC-01/C-1/C-2):** Motor tüm yönetim uçlarında `Authorization: Bearer <DCA_API_KEY>` zorunlu tutar (yalnız `/api/health` açıktır). Yerel başlatıcı ilk çalıştırmada rastgele `DCA_API_KEY`, `DCA_VAULT_KEY` ve ayrı `DCA_SESSION_SECRET` üretir; üçünü de `.local-runtime/local-keys.env` içinde saklar. UI oturum açma için parola (`DCA_UI_PASSWORD`) ve en az 32 baytlık imza anahtarı (`DCA_SESSION_SECRET`) ister; Docker dağıtımı bu sırları ayrı volume'da kalıcı üretir. Yerel tek-kullanıcı arayüzü yalnız `127.0.0.1` üzerinde dinler; isteğe bağlı `DCA_UI_PASSWORD` ile giriş koruması açılabilir. Faz 21 (P0-9): UI anahtarı ASLA gömmez — tüm motor çağrıları aynı-origin `/api/engine/*` middleware proxy'sinden geçer ve Bearer YALNIZ sunucu tarafında (`ENGINE_API_KEY` env) eklenir. SSE (`EventSource` başlık koyamaz) `/api/stream` route handler'ı üzerinden sunucu tarafında proxy'lenir. `NEXT_PUBLIC_ENGINE_API_KEY` adı kullanımdan kaldırıldı (paket sızıntı tuzağı). Anahtarlar yerel çalışma içindir, depoya girmez.

UI parola denemeleri, yalnız `DCA_TRUSTED_CLIENT_IP_HEADER` açıkça ayarlanmışsa o başlıktaki geçerli IP'ye göre ayrı ayrı sınırlandırılır; aksi halde yerel tek-kullanıcı kovası kullanılır. Bu başlığı yalnız dışarıdan gelen değeri kendi ağına göre doğrulayıp yeniden yazan güvenilir ingress için ayarlayın. Docker Compose, loopback'e bağlı servisin Cloudflare Tunnel arkasında kullanılması için `CF-Connecting-IP` varsayılanını kullanır; başka bir ingress'te değiştirin veya boş bırakın. Yatay/çoklu süreç kurulumlarında bu bellek içi sınırlayıcı yerine ingress'te ortak rate limit gerekir.

Yerel çalıştırma `.local-runtime/paper.sqlite3` veritabanını kullanır; mevcut `db/dca-core.sqlite3` dosyasındaki botları veya anahtarları yüklemez. Bybit ve Binance testnet, OKX demo modu etkin başlar. Özel endpoint değerleri exact-host allowlist ve mainnet kapılarından geçer. Gerçek hesap anahtarı eklemeyin; yerel başlatma betikleri PAPER veritabanıyla çalışır. Motor 3002 portunu yalnız 127.0.0.1 üzerinde dinler.

Günlükler `.local-runtime/` içindedir. Durdurmak için:

```powershell
npm run local:stop
```

Yerel PAPER veritabanı durdurmada korunur. Geliştirme sunucusu üretim dağıtımı için kullanılmamalıdır.

### Bot oluşturma API'si — örnek gövde (D-1)

`POST /api/bots` gövdesinde `config`, `BotConfig`'in serde şemasının TAMAMINI taşımalıdır (`kind` ve `symbol` üst düzey alanlarla çiftlenir). Çalışan en küçük örnek:

```json
{
  "name": "ornek-bot",
  "kind": "DCA_LONG",
  "venue": "BYBIT",
  "mode": "PAPER",
  "symbol": "BTCUSDT",
  "interval": "1h",
  "start_cash": "1000",
  "config": {
    "kind": "DCA_LONG", "symbol": "BTCUSDT",
    "base_order_size": "50", "safety_order_size": "25",
    "sizing_mode": "QUOTE_NOTIONAL",
    "deviation_pct": "0.005", "step_scale": "1.2", "volume_scale": "1.1",
    "max_safety_orders": 3, "target_profit_pct": "1", "use_net_tp": true,
    "trailing_enabled": false, "trailing_trigger_pct": "1", "trailing_deviation_pct": "0.5",
    "stop_loss_pct": null, "leverage": "1",
    "hedge_ratio": "0.5", "combined_target_profit": "50",
    "grid_lower": null, "grid_upper": null, "grid_levels": null,
    "grid_mode": null, "grid_trail": false, "cooldown_bars": 0,
    "entry_trigger": "INSTANT"
  }
}
```

Dikkat: `deviation_pct` KESİR olarak yazılır (`0.005` = %0,5); `start_cash` pozitif olmalıdır. Hatalı kombinasyonlar (örn. kümülatif sapmanın %100'ü aşması) kurulumda 422 ile reddedilir.

> **Sürüm:** 3.0-CANONICAL-UNIFIED
> **Mimari:** Rust High-Performance Quantitative Engine (`dca-core`) + Next.js App Router Quantitative Terminal
> **Temel İlkeler:** Sıfır-Float Rasyonel Aritmetik (M01–M21), 8 Fail-Closed Risk Kapısı, Sıfır-Emir-Yetkili AI, Sıfır-Çekim-Yetkili Vault, Tek Süreç / Düşük Kaynak Dağıtımı

---

## 1. Mimari Genel Bakış

```
┌────────────────────────────────────────────────────────────────────────────────┐
│                  NEXT.JS 16 QUANTITATIVE TRADING TERMINAL                      │
│   Kokpit / Dashboard • 5-Adım Bot Yaratma Sihirbazı • 7-Sekmeli Bot Stüdyosu   │
│   İnteraktif DCA/Grid Hesaplayıcı • Backtest Lab • AES-256-GCM Vault Ayarları   │
└──────────────────────────────────────┬─────────────────────────────────────────┘
                                       │  Gateway (XTransformPort=3002) / SSE
                                       ▼
┌────────────────────────────────────────────────────────────────────────────────┐
│                       RUST QUANTITATIVE CORE ENGINE  (port 3002)               │
│          Safe Rust • Tokio Async Event Loop • SQLite WAL Persistence           │
│                                                                                │
│  ┌────────────────────────┐ ┌────────────────────────┐ ┌────────────────────┐  │
│  │     dca_math Crate     │ │    dca_kernel Crate    │ │ dca_strategy Crate │  │
│  │ - 4096-bit Rational    │ │ - 8 Fail-Closed Gates  │ │ - DCA Long / Short │  │
│  │   Fraction Arithmetic  │ │ - Single Event Kernel  │ │ - Spot/Fut Grid    │  │
│  │ - Half-Even Rounding   │ │ - SimulationStepper    │ │ - Hedge Groups     │  │
│  │ - WAC Accounting       │ │ - M16 Order Reconciler │ │ - Combo & Infinity │  │
│  │ - M01–M21 Normative    │ │ - Liquidation Monitor  │ │ - Sizing Engine    │  │
│  └────────────────────────┘ └────────────────────────┘ └────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────┐  │
│  │                   dca_exchange & dca_risk Crates                         │  │
│  │  - BYBIT v5 Adapter (ÖNCELİKLİ: REST + HMAC imza + WS hazır)             │  │
│  │  - Binance Adapter (ikincil) + Paper Venue                               │  │
│  │  - Zero-Withdrawal Hard Gate (HTTP 422) & AES-256-GCM Vault              │  │
│  │  - M21 NYSE Paskalya Takvimi + 23/5 Emtia + 24/7 Kripto Seansları        │  │
│  └──────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────┘
```

## 2. Dizin Yapısı

```
.
├── src/app/                         # Next.js App Router (Self-hosted offline fontlar, error/loading sınırları)
├── src/components/terminal/         # Kokpit, Filo, Stüdyo, Yeni Bot, Hesaplayıcı, Backtest Lab, Ayarlar
├── src/hooks/use-engine.ts          # engineFetch (/api/engine/* proxy), SSE store, polling hooks
├── src/lib/engine-types.ts          # Rust motor wire tipleri (exact decimal strings)
├── src/lib/format.ts                # fmtFrac / fmtUsd / fmtTs — half-even display yardımcıları
├── Dockerfile                       # Multi-stage üretim Docker imajı (Rust + Node standalone)
├── docker-compose.yml               # Tek komutla orkestrasyon ve SQLite kalıcı veri hacmi
├── scripts/                         # Standalone postbuild, backup, yerel yönetim scriptleri
├── docs/BYBIT_RESEARCH.md           # Bybit v5 derin araştırma (canlı doğrulanmış)
└── mini-services/dca-core/          # RUST KANTİTATİF MOTOR (Cargo workspace, 8 crate, 296 birim test)
    ├── crates/dca_math/             # M01–M21: Fraction 4096-bit + tüm normatif formüller
    ├── crates/dca_kernel/           # Bar, OrderIntent, SimulationStepper (tek olay çekirdeği)
    ├── crates/dca_risk/             # 8 Fail-Closed Gate saf fonksiyonları + portfolio daily loss cap
    ├── crates/dca_strategy/         # DCA/Grid/Infinity/Hedge motorları + backtest & LIVE certify gate
    ├── crates/dca_exchange/         # Bybit v5 (fail-closed mainnet gate), Binance, OKX, Paper venue
    ├── crates/dca_signals/          # İndikatör motoru (RSI/MACD/BB/ATR/ADX/Supertrend) + sinyal ağacı
    ├── crates/dca_pine/             # Pine Script v5 yorumlayıcı ve AST derleyici
    └── crates/dca_server/           # axum REST+SSE :3002, SQLite WAL (busy_timeout), vault, /metrics
```

## 3. Çalıştırma Seçenekleri

### A. Docker ile Tek Komutta (Önerilen Üretim Dağıtımı)

```bash
docker compose up -d
```
Arayüz `http://localhost:3000`, motor `http://localhost:3002/api/health` ve Prometheus metrikleri `http://localhost:3002/metrics` adresinde anında aktif olur.

Docker dağıtımında SQLite verisi `dcabot-data`, API/vault/oturum imza anahtarları ve otomatik UI parolası `dcabot-secrets` adlı ayrı volume'da saklanır. Otomatik UI parolası loglara yazılmaz; yerel yönetici parolayı `docker exec dcabot-terminal cat /app/secrets/.ui_password` ile okuyabilir. Harici secret mount kullanırken `DCA_API_KEY_FILE`, `ENGINE_API_KEY_FILE`, `DCA_VAULT_KEY_FILE`, `DCA_SESSION_SECRET_FILE` veya `DCA_UI_PASSWORD_FILE` değişkenini container içindeki dosya yoluna ayarlayın; dosya eksik/boşsa container güvenli biçimde başlamaz. Yedeklemede iki volume'u birlikte koruyun: secrets volume'u kaybolursa UI oturumları geçersizleşir, vault volume'u kaybolursa SQLite içindeki şifreli borsa anahtarları çözülemez. Eski kurulumlardaki `/app/data/.api_key` ve `.vault_key` dosyaları ilk açılışta doğrulanarak secrets volume'una taşınır; eski volume izinleri silmeye engel olursa dosyanın içeriği temizlenir.

### B. Yerel Geliştirme (Windows / Linux / macOS)

```bash
# Motor testlerini çalıştır (296 test)
cargo test --manifest-path mini-services/dca-core/Cargo.toml

# Motoru başlat (Port 3002)
cargo run --manifest-path mini-services/dca-core/Cargo.toml --bin dca_server

# Arayüzü başlat (Port 3000)
npm run dev
```

Port sabit: **3002** (Motor) ve **3000** (Next.js UI). UI tüm motor çağrılarını sunucu tarafında `/api/engine/*` proxy'si üzerinden yetkilendirerek iletir.

## 4. Matematiksel Çekirdek (M01–M21)

| Modül | İçerik | Test |
|---|---|---|
| M01 | 4096-bit rasyonel kesir; `gcd=1`, `den>0`; NaN/Inf reddi; half-even display (yalnız raporlama sınırında) | 13 |
| M02 | DCA sabit seviye planı; kümülatif sapma `D_i = d·Σa^k`; tickDown/Up hizalama | 6 |
| M03 | Plan notional, initial margin, stres teminatı (Ω şok yolları) | — |
| M04 | WAC muhasebe; kısmi kapanışta ortalama SABİT; `fill_key` idempotensi; overfill ihlal kaydı | 6 |
| M05 | Ücret/funding; exact net TP: Long `(qA+F+T)/(q(1−f))` tickUp, Short `(qA−F−T)/(q(1+f))` tickDown; funding işaret köprüsü | 3 |
| M06 | Equity, unrealized PnL, HWM/DD/MDD, TWR | 1 |
| M07 | Exact likidasyon sınırı; MMR bracket iterasyonu: Long `(qA−W−d)/(q(1−r))`, Short `(qA+W+d)/(q(1+r))` | 2 |
| M08 | Tick/step ızgarası; qtyDown; intrabar penetrasyon kuralları | — |
| M15 | Aritmetik/geometrik mesh; geometrik mesh **BigInt tam n'inci kök** (sıfır float!); fee-breaker `g_min=(f_b+f_s)/(1−f_s)` | 4 |
| M16 | Order reconciliation: minimal diff (cancel→place sırası, sıfır duplikat) | 2 |
| M17 | Infinity grid: `V_base` sabit; `Δq = q − V/(p+Δp)` | 1 |
| M18 | Hedge coupling: `Δ_net = q_main − ρ·q_hedge`; combined exit | 1 |
| M20 | SL–merdiven çakışma dedektörü: `SL_trig(A_k) ≥ P_{k+1}` ulaşılır | 2 |
| M21 | NYSE tatil takvimi (Meeus/Jones/Butcher Paskalya), EDT/EST, 23:00–22:00 UTC emtia, borç faizi N·r/360 | 5 |

**Toplam: 296 birim test (8 crate) — hepsi yeşil.** Float tipi yalnızca raporlama sınırında (Sharpe display) kullanılır.

## 5. 8 Fail-Closed Risk Kapısı

1. **GATE1 Veri Bütünlüğü** — OHLCV yapısal geçerlilik + monoton zaman damgası; ihlalde **MOTOR DURUR**
2. **GATE2 Ücret Kesintisi** — sıfır ücret fill doğrulanmış promosyon yoksa reddedilir
3. **GATE3 Kayma Cezası** — market emirleri tetik fiyatından derefilledemez: `P_fill = P_open·(1±slip)`
4. **GATE4 Sıfır Look-Ahead** — bar kapanışında alınan karar en erken t+1'de çalışır
5. **GATE5 Limit Gerçekçiliği** — `MARKETABLE_TAKER / RESTING_TOUCH / OPEN_GAP_BETTER` etiketleri + bar hacminin %10'unu aşan fill yok
6. **GATE6 Sermaye Koruması** — nakit/marj yetersizde SKIP/WAIT; negatif nakit asla
7. **GATE7 Seans Koruması** — piyasa kapalıyken ADD reddedilir (FAIL_CLOSED), TP/SL/REDUCE korunur
8. **GATE8 Gap Koruması** — 09:30–09:35 ET açılış salvosunda safety emirleri ertelenir; çoklu seviye gap'lerinde tek sentetik VWAP fill

## 6. Bybit Entegrasyonu (Öncelikli Platform)

`docs/BYBIT_RESEARCH.md` — canlı `curl` doğrulamalı tam araştırma:
- **İmza:** `HMAC-SHA256(ts + api_key + recv_window + queryString|jsonBody)` → lowercase hex
- **Header'lar:** `X-BAPI-API-KEY, X-BAPI-SIGN, X-BAPI-TIMESTAMP, X-BAPI-RECV-WINDOW`
- **İzin taraması:** `GET /v5/user/query-api` → `permissions.Wallet` dizisinde `"Withdraw"` varsa **HTTP 422 HARD FAIL** (sıfır-çekim kapısı)
- **Enstrümanlar:** `priceFilter.tickSize`, `lotSizeFilter.qtyStep/minOrderQty/minNotionalValue` (spot+linear)
- **Kline:** `result.list` DESC → kernel'e ASC normalize
- **Fee:** spot 0.10%/0.10%, linear maker 0.020%/taker 0.055% (varsayılan kademe)

## 7. API Kontratı (dca-core :3002)

| Yöntem | Yol | Açıklama |
|---|---|---|
| GET | `/api/health` | motor durumu |
| GET | `/api/summary` | NAV, exposure, allocation, fleet sayıları |
| GET/POST | `/api/bots` | fleet listesi / bot oluştur (M20 + fee-breaker pre-flight) |
| GET/PATCH/DELETE | `/api/bots/:id` | detay (ladder dâhil) / dinamik TP-SL-trailing patch / durdur |
| POST | `/api/bots/:id/actions` | START/PAUSE/STOP/CLOSE_ALL/ADD_FUNDS |
| GET | `/api/bots/:id/fills` | fill kayıtları (role, fee, realism etiketi) |
| GET | `/api/bots/:id/events` | kernel olay günlüğü |
| POST | `/api/calc/dca` | DCA merdiven hesaplayıcı + M20 + likidasyon |
| POST | `/api/calc/grid` | Grid mesh hesaplayıcı + fee-breaker |
| POST | `/api/backtest` | 1000 bar'a kadar backtest (equity curve, MDD, win rate) |
| GET | `/api/markets` | enstrüman önbelleği (Bybit canlı) |
| GET | `/api/klines` | mum verisi (Bybit canlı) |
| POST/GET/DELETE | `/api/vault/keys` | AES-256-GCM şifreli anahtar kasası + zero-withdrawal gate |
| GET | `/api/stream` | SSE: `FILL`, `EQUITY`, `LOG`, `BOT_STATE`, `ORDER`, `PING` |

## 8. Güvenlik

- **Zero-Withdrawal Hard Gate:** çekim yetkili anahtar anında 422 ile reddedilir — aşılamaz
- **AES-256-GCM Vault:** anahtarlar 12 bayt rastgele nonce ile şifrelenir; master key zorunlu `DCA_VAULT_KEY` ortam değişkeninden gelir ve SQLite'a yazılmaz. Yeni kayıtlar Argon2id tabanlı, sürümlü zarf kullanır; eski kayıtlar geçiş için okunabilir.
- **Sıfır-Float:** tüm fiyat/miktar/ücret/PnL hesapları `Fraction` (BigInt num/den); JSON'da exact `"n/d"` string
- **Fail-Closed:** belirsiz/eksik durumda emir ASLA gönderilmez
- **Portföy Notional Kapısı (Faz 27/İ-1):** filo-geneli brüt açık notional + aday
  emir, toplam sermayenin `DCA_PORTFOLIO_MAX_NOTIONAL_PCT` (varsayılan **%85**,
  `0` = kapalı) yüzdesini aşacaksa Add emri yerleştirilmez; WARN olayı yayınlanır.
  Kapanış (TP/SL/CLOSE) emirleri muaf — çıkış yolu her zaman açık.
- **Dışa Açılım (Tunnel) Uyarısı (Faz 27/S-2):** servisler yalnız `127.0.0.1`
  dinler. Cloudflare Tunnel ile dışarı açarken **yalnız UI (:3000) hedeflenmeli**;
  motor (:3002) asla doğrudan tünellenmemeli. UI'nin `/api/stream` uçları sunucu
  tarafı proxy'den geçer; motorun anahtarsız `/api/health` ve `/api/stream`
  uçları tünel arkasında erişilebilir olur — tunnel kural dosyasında bu iki yol
  için ek kimlik doğrulama (Cloudflare Access) zorunludur.

## 9. Bilinen Kısıtlar / Yol Haritası (Horizon 5)

- ✅ Horizon 1: Pure Math & Kernel (M01–M21 + 8 Gate)
- ✅ Horizon 2: Strategies & Backtesting
- ✅ Horizon 3: Live Connectors (Bybit REST tam; WS feed modülü hazır — private WS aboneliği testnet doğrulaması bekliyor) + Vault + SQLite WAL
- ✅ Horizon 4: Next.js Terminal (tek-route kısıtı: tüm görünümler client-side nav ile `/` altında)
- ⏳ Horizon 5: 6 aylık testnet rampası — 20/20 testnet checklist + dual-run parity audit (TS referans motoru yokluğunda Rust-dahili parity: backtest vs paper replay)

## 10. Sandbox Notu

Bu kısıtlı bulut ortamında `next dev` soğuk derlemesi ~2GB tepe bellek kullanır; paylaşımlı node'da global OOM baskısı dönemsel olarak derlemeyi keser. Rust çekirdeği (`dca_server`, ~7MB RSS) bu kısıttan bağımsız stabil çalışır. UI derlemesi bir kez tamamlandığında `.next` önbelleği sıcak kalır.
