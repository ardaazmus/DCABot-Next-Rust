# ENVANTER — H:\DCABOT-v3 → Bu Platform Port Listesi

> Kaynaklar: `H:\DCABOT-v3\docs\DCABOT_MASTER_SYSTEM_INVENTORY_AND_CAPABILITY_MATRIX.md` (SPEC-2026) + iki projenin **canlı kod taraması** (2026-10-22).
> Lejant: **VAR** = burada çalışıyor, doğrulandı · **KISMEN** = çekirdek var, yüzeyi eksik · **YOK** = port adayı (önceliği yanında).
> Öncelikler: **P0** kritik boşluk · **P1** yüksek değer · **P2** orta · **P3** düşük-orta · **P4** fikir/lüks.

## 0. Boyut karşılaştırması (ham sayım, tahmin değil)

| Ölçü | v3 (H:\DCABOT-v3) | Bu platform | Not |
|---|---|---|---|
| API endpoint dosyası (`route.ts`) | **71 dosya, 26 grup** | — (Next katmanında API yok) | Bizde Next sadece arayüz; tüm API Rust motorda |
| Motor endpoint'i (axum `.route`) | — (TS route'ları) | **25 route** | `/api/health` + 24 işlev |
| UI sayfa/görünüm | **29 sayfa** (`(app)` + classic + login + piyasa) | **8 görünüm** (Kokpit/Filo/Studio/Wizard/Hesaplayıcı/Backtest/Bilgi/Ayarlar) | Tek-terminal kabuk bilinçli tercih; sayfa çokluğu ≠ özellik çokluğu |
| UI bileşeni (`.tsx`) | **128 dosya** (dashboard ~60, ui ~40) | **14 terminal + ui paketi** | |
| Rust crate | yok (TS motor) | **8 crate** (`dca_math/kernel/strategy/signals/pine/risk/exchange/server`) | Matematik burada 4096-bit rasyonel |
| Matematik modülü | M01–M20 (TS) | **M01–M20 (Rust)** + Gate 1-8 | Birebir+ |

v3'teki "36 endpoint" ifadesi master matrix'in çekirdek sayımıdır; diskte 71 route dosyası var (alt yollar: `bots/[id]/grid`, `calculator/brackets/*`, `market/*` (11), `live/*` (11), `account/*` (7) vb.). Aşağıdaki tablo **71 dosyanın tamamını** kapsar.

## 1. Matematik çekirdeği (M01–M20) + kapılar

| v3 | Bu platform | Not |
|---|---|---|
| M01–M08 rasyonel çekirdek, anti-halüsinasyon kapıları | **VAR** | `dca_math`/`dca_kernel`, 4096-bit exact |
| M09–M20 (SL-merdiven, g_min, seans, salvo, gap, makro, rejim, DD kesici, dolum) | **VAR** | RiskPolicy ile bot başına ZORUNLU/UYARI+İZİN (Wizard Bölüm 6) |
| Seans kontrolü + Gap koruması (Gate 7-8) | **VAR** | SessionType + grid gap kapıları |
| Gate 1-6 fail-closed (veri/fee/slippage/lookahead/realism/sermaye) | **VAR** | Emir üretilmez, SKIP/WAIT |
| Zero-withdrawal hard gate (çekim izinli anahtar 422) | **VAR** | Birebir; self-check 14/19 her çalışta denetler |
| VaR99/CVaR99 filo riski | **VAR** | `GET /api/fleet/risk` (σ_p/VaR99/CVaR99/DR + excluded + partialCoverage; f64 tahmin etiketli, v3 formülleriyle birebir; Faz 2.1) + `risk-warnings.ts` + filo kartları |

## 2. Motor & yürütme (v3 `bots/*`, `deals/*`, `live/*`)

| v3 | Bu platform | Not |
|---|---|---|
| Tek çekirdek (backtest=paper=live) | **VAR** | Aynı motor; paper botlar canlı |
| DCA Long/Short, Futures DCA, Spot/Futures Grid, Infinity Grid | **VAR** | + HEDGE_PROTECTIVE/DUAL |
| Bot aksiyonları (start/pause/stop/close-all/scale/freeze) | **VAR** | `START/PAUSE/STOP/CLOSE_ALL/ADD_FUNDS/FREEZE/UNFREEZE`; scale v3'teki gibi ayrı endpoint değil, ADD_FUNDS ile (KISMEN) |
| Freeze (yeni emir durur, açıklar korunur) | **VAR** | Per-bot `FROZEN` + **global** `/api/live/freeze` (LIVE hat; gövdesiz POST toggle, v3 uyumu) + Dashboard bandı + SSE; restart-safe (olaydan geri yüklenir) |
| Kill-switch (çift onay) | **VAR** | **Global** `/api/live/killswitch` (engage/release + TV ENTRY dahil yürütme durur, restart-safe) + KillButton (2-adımlı engage, bırakma + selfcheck toastı) + Dashboard bandı + akış rozetleri + SSE; eski istemci-döngüsü stop-all kaldırıldı (v3 semantiği) |
| Grid canlı görünüm/fill'ler (`grid-live`, `grid/fills`) | **VAR** | `/api/bots/:id/grid` (mesh+armed) + tick + Studio Grid Mesh paneli (P3-8) |
| Deals tick + multi-pair deals | **VAR** | `/bots/:id/tick` (manuel poll turu) VAR; Filo'da "Son Dolumlar (tape)" paneli (`/api/fleet/overview` tape, son 40 fill — Faz 2.8) |
| Combo preview (çoklu-bot paketi) | **YOK** | COMBO preset motorda yok — taşınmaz (Faz 2.8 hükmü) |
| Hedge grupları (UI: liste/detay/sihirbaz) | **KISMEN** | Motor `hedge.rs` VAR + Studio Hedge sekmesi VAR; Filo'da 🛡 Hedge filtre rozeti (istemci-taraflı — Faz 2.8); ayrı grup sayfası yok (bilinçli) |
| Bot hafızası (`memory`) | **VAR** | `GET/POST /api/bots/:id/memory` (`bot_memories` + ABSTAIN sözleşmesi) + Studio Hafıza kartı; otomatik-tetikleme yok (Faz 2.3) |
| Dry-run (0-risk filtre dogrulama) | **VAR** | `POST /api/live/dryrun` (M08 hizalama, sent:false, fail-closed) + Studio Ozet'te 0-Risk Dene paneli; Binance exchangeInfo sembol-filtreli + venue-nitelikli onbellek (Faz 1.1) |
| Multi-status (uc venue saglik) | **VAR** | `GET /api/live/multi-status` (sonda + anahtar VARLIGI + OKX posMode + venue-bazinda bot/reconcile destegi + bakim sentineli; Faz 1.3) |
| Filo kokpiti / executive overview | **VAR** | `GET /api/fleet/overview` (NAV/kaldirac/net/unrealized/winRate + venue tahsis + tape(40) + aciklar + makro + sentiment; 24s/7g + PF durust `null` — deals defteri yok; Faz 1.4) |
| Risk-budget kartı | **KISMEN** | RiskPolicy + filo uyarıları + Lab VaR paneli var; ayrı bütçe kartı yok — taşınmaz (Faz 2.8 hükmü) |
| Cancel-all + status sayfaları | **KISMEN** | CLOSE_ALL aksiyonu var; ayrı sayfa yok (gerekmez — karttan yapılır) |
| Reconcile (borsa↔yerel mutabakat) | **VAR** | `/api/live/reconcile` + Studio Mutabakat paneli; raporlama-odaklı (otomatik durdurma yok — bilinçli); PAPER'da supported:false |
| LIVE sertifika kapısı (SonCert) | **VAR** | Faz 0.1: `mode==LIVE` create → motor-içi inline sertifika (trades≥5, WFE≥%50, DSR≥0.95, PBO≤%20, vadeli funding-şartı); KALDI → 422 `CERT_GATE_FAILED` + kapı detayı; `/api/backtest/cert` yanıtında `verdict+gates`; Lab'da hüküm rozeti (v3 §10 kuralının API-seviyesi karşılığı) |
| Self-check (§11.2-2, **20 kontrol**) | **VAR** | **19 kontrol** port edildi (`/api/live/selfcheck` + Ayarlar paneli); eksik 1 = **test-order bacağı bilinçli YOK** (ağa emir gönderilmez); erişilemeyen halka-açık venue FAIL değil ABSTAIN; verdict GECTI/KALDI/YETERSIZ_ORTAM; SELF_CHECK olayı append-only |
| Pine Script alt kümesi + kütüphane + webhook | **VAR** | `when=` dahil; `strategy.order` yok (bilinçli); `/api/webhooks/tradingview` (sabit-saat secret + ±60sn replay penceresi) + Ayarlar'da TV kurulum kartı (yol + şablon + fail-closed kuralları — Faz 2.8) |
| Sinyal builder preview | **VAR** | `/api/signals/preview` + Studio signal-builder |
| Backtest nedenleri (no-deal) | **VAR** | v3'te yok — burası ileride |
| WFE/DSR/PBO sertifika + rozet | **VAR** | `certify.rs` + Lab rozetleri |
| Monte Carlo + realism karşılaştırma | **VAR** | v3'te yok — ileride |
| Parity (backtest↔paper sapma rozeti) | **VAR** | `/api/parity` (≥%90 eşik) + Studio rozeti; paper<10 fill → YETERSIZ_VERI (fail-closed) |
| Kalibrasyon grid-search | **VAR** | `/api/analysis/calibration` (μ/σ/q95 + öneri) + Lab Öner düğmesi (P3-3) |
| Rejim analizi (ADX/ATR/Hurst) + rozet | **VAR** | `/api/analysis/regime` (motor classify) + Lab rozeti SAKİN/HAREKETLİ/ÇÖKÜŞ (P3-2) |
| Olay adli tıp (trace şelale) + forensics | **VAR** | `GET /api/events/forensics` (trace/traces/filtre) + Studio Olaylar'da Tick İzleri (zaman çizgisi); tick başına `trace_id` (göçlü şema). Replay modal yok (bilinçli). Span kırılımı yok (Faz 2.4) |
| AI teşhis/copilot (salt-okunur) | **YOK** | **P4** (harici anahtar maliyeti var) |

## 3. Piyasa verisi (v3 `market/*` 11 endpoint, `macro`, `indicators`)

| v3 | Bu platform | Not |
|---|---|---|
| Sembol tarayıcı + Backtest Lab sembol seçici | **VAR** | `/api/markets` (Bybit sayfalamalı 907 linear/527 spot, Binance, OKX spot+SWAP); Wizard + Lab dropdown |
| Ticker 24s | **VAR** | `/api/tickers` (30sn cache) + Wizard rozetleri; PAPER dürüst boş |
| Kline önbelleği | **VAR** | Bybit+Binance+OKX (OKX bar adı çevrimi 60→1H), instruments cache |
| Orderbook L2 + imbalance | **VAR** | `/api/orderbook` (Bybit + Binance spot; REST anlık, WS yok — dürüst sınır) + Lab Derinlik satırı (P3-6) |
| Market pulse / feeds / ping / refresh / symbols | **KISMEN** | symbols+refresh karşılığı `/api/markets`; `GET /api/market/ping?venue=` VAR (4sn timeout + base açıklaması; Faz 1.5); pulse/feeds akışı yok (P3) |
| Haber motoru (news/news-ai/news-board) | **VAR** | Motor news/news-ai + Dashboard açılış "Piyasa Nabzı" (sentiment rozeti, delisting alarmı, TR-özet); news-board akışı yok |
| İndikatör endpoint'i (`indicators`) | **VAR** | `GET /api/indicators?set=` (EMA/SMA/RSI/BB/MACD; sinyal-motoruyla aynı fns; MACD quirk düzeltilmiş + kilit testli; Faz 2.2) |
| Makro takvim/kara pencere (`macro/state`) | **VAR** | Motor filtresi + `GET /api/macro/state` (aktif/pencere/sonraki olay; tier alanı yok — Faz 1.5) |
| Bakım sentineli + banner (`system/maintenance`) | **VAR** | `/api/system/maintenance` (canlı sonda + QUIET bayrağı + `simulated`) + `POST .../simulate` (yalnız `DCA_ALLOW_SIMULATE=1`, bellek-içi; Faz 1.5) + üst bant + Ayarlar paneli (P3-5) |
| BNB komisyon monitörü | **VAR** | `GET /api/live/bnb` (serbest BNB + 0.05 eşiği, 45 sn önbellek; anahtarsızda ABSTAIN) + Ayarlar kartı; süpürme (POST) bilinçli YOK (Faz 1.6) |

## 4. Güvenlik, kasa, bağlantılar (v3 `api-keys`, `venues`, `account/*`, `auth`, `blacklist`)

| v3 | Bu platform | Not |
|---|---|---|
| AES-256-GCM vault + zero-withdrawal 422 | **VAR** | Birebir |
| Venue kayıt defteri + kapı (requireVenue/VR2) | **VAR** | Bybit+Binance+OKX+Paper adapter'ları VAR (P4-3d); v3'teki venue-gate sarmalı ayrı katman olarak yok |
| Anahtar doğrulama + bağlantı kartları | **VAR** | Self-check 12-15/18-19 her çalışta dener (trade izni, withdraw yasağı, cüzdan, pozisyon) |
| Kullanıcı girişi (login/logout) | **YOK** | Bilinçli: tek-kullanıcı yerel terminal; çok-kullanıcıda **P2'ye yükselir** |
| Kara liste (blacklist) sembol filtresi | **VAR** | `pair_blacklist` + CRUD + create_bot 422 kapısı + Ayarlar paneli (P3-1) |
| Bellek sıfırlama (zeroization) | **VAR** | `zeroize` zarfı imza anahtarlarında + scrub self-testi (P3-4) |
| Telegram webhook | **YOK** | P4 |
| Futures hesap kartları (check/reconcile/state/symbol) | **VAR** | `/api/live/futures-state` (kurallar + canlı pozisyon, salt-okunur; BYBIT+OKX) + Lab FUT. DCA kartı (P4-3d: venue geçişli) |

## 5. UI görünümler (v3: 29 sayfa; burada: 8 görünüm)

| v3 sayfası | Bu platform | Not |
|---|---|---|
| Kokpit/dashboard, Bot Filosu, bot detay, Yeni Bot sihirbazı | **VAR** | Dashboard/Fleet/Studio/Wizard |
| Hesaplayıcı (+venue kademe aynası), Backtest Lab (+certify) | **VAR** | Kademe aynası VAR (`GET /api/calc/brackets`: Bybit public + Binance imzalı; manuel giriş oturum-içi, sunucuya yazılmaz — bilinçli); backtest/cert venue-parametreli (BYBIT/BINANCE/OKX kline+funding+kurallar, yanıt `venue` ekolu) (P4-3d, Faz 2.7) |
| Ayarlar (kasa, venue, blacklist, position-mode) | **VAR** (+self-check paneli) | Blacklist satırı YOK (P3) |
| Ops: live, kill-switch, events | **VAR** | Kill kart + event listesi; ayrı ops sayfaları yok (gerekmez) |
| Piyasa istasyonu (markets, piyasa, classic) | **YOK** | P3 |
| Yardım: docs/glossary/knowledge/assistant | **VAR** | Bilgi sekmesi: SSS (kategori sekmeli) + Sözlük + Türkçe-duyarsız arama + YZ paneli bağlantısı (Faz 2.5) + Sözleşme sekmesi: dürüstlük paneli + M01–M08 + yol haritası + docs/ listesi (Faz 2.6; v3 ADR listesi uydurulmadan) |
| Entegrasyon sayfaları (API/TV) | **KISMEN** | Vault + TV webhook VAR; tanıtım sayfası yok (gerekmez) |
| Login | **YOK** | Bilinçli (bkz. §4) |
| Stream (SSE endpoint) | **VAR** | Motor SSE veriyor; Next proxy'si üzerinden tüketiliyor |

## 6. Bu turda yapılanlar (P0–P2)

- [x] P0 Backtest Lab borsa-içi sembol seçici (907 linear / 527 spot, sayfalamalı) + Wizard dropdown sadeleştirme.
- [x] P1 Parity + Reconcile canlı doğrulandı (fail-closed notlarıyla).
- [x] P2 Ticker 24s (`/api/tickers` + Wizard rozetleri, PAPER dürüst boş).
- [x] P2 Freeze (`FROZEN` + Filo ❄ Dondur/Çöz; 4 bot RUNNING, pozisyonlar korunarak doğrulandı).
- [x] P2 Self-check: `/api/live/selfcheck` (**19 kontrol**: 6 Yerel + 5 Ortam + 6 İmza/Bybit + 2 İmza/Binance) + Ayarlar paneli; canlı sonuç **✓ GECTI — 19 geçti · 0 kaldı · 0 atlandı** (Bybit+Binance anahtarlı ortamda; anahtarsızda imzalılar ABSTAIN → YETERSIZ_ORTAM). Test-order bacağı bilinçli yok.
- [x] Bu envanterin düzeltilmesi (36→71 endpoint sayımı, 29 sayfa/128 bileşen sayımı, eksik satırlar: auth/blacklist/combo/deals/grid-live/indicators/macro-state/pulse/login).

## 7. Sıradakiler (öneri)

- **P3 hepsi tamamlandı:** kara-liste · rejim · kalibrasyon · zeroization · bakım bandı · orderbook · futures kartı · grid mesh+tick (bu tur). Kalan: P4 fikirler (haber, AI copilot, OKX, Telegram, yardım sözlükleri, combo-görünüm).
- **P4 (fikir):** haber motoru, AI copilot, OKX, Telegram, yardım sözlükleri UI.
- **Not:** çok-kullanıcı ihtiyacı doğarsa login/auth P2'ye yükselir; o güne kadar YOK durumu bilinçli tercihtir.

## 8. Basit dille özet (teknik olmayan okur için)

v3'teki **her şeyin motor karşılığı burada çalışıyor**: alım-satım matematiği, 8 güvenlik kapısı, 20 risk modülü, grider, DCA'lar, hedge, test-araçları (backtest/parite/sertifika), borsa bağlantıları ve kasa. Ekranda ise v3'ün 29 sayfası yerine **8 sade ekran** var — özellik kaybı değil, sadeleştirme (8. ekran: Bilgi Merkezi). Bu tur eklenen **Self-Check**, tek tuşla 19 maddelik sağlık raporu veriyor ("borsa bağlı mı, anahtar yetkili mi, çekim kapalı mı, botla borsa aynı fikirde mi") ve şu an **hepsi yeşil**. Eksikler (kara liste, haberler, yapay zekâ yardımcısı gibi) parayı ve güvenliği **etkilemeyen** konfor özellikleri; listede sırayla duruyor.
