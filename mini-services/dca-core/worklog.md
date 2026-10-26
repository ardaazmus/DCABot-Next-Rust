
---

Task ID: faz-1-6-bnb + faz-2-6-sozlesme (2026-10-22)
Ne: v3 BNB monitor GET portu + Sozlesme sekmesi (Bilgi Merkezi 3. sekme). Ikisi tek oturumda.
Motor (1.6): yeni `dca_server/src/bnb.rs` - `GET /api/live/bnb`: imzali Binance spot
`GET /api/v3/account` uzerinden serbest BNB + esik `0.05` (v3 `lt` aynen) + 45sn bellek-ici
onbellek (`AppState.bnb_cache`; yalniz EXCHANGE basarisi yazilir). Anahtar yok/cagri
basarisiz/zaman asimi (8sn) -> ABSTAIN (`freeBnb:null` + sebep); ASLA 500 degil. POST
supurme TASINMADI (R.2). Bilincli sapma (kodda notlu): v3 `BNB_LOW_ALERT` olayini 30dk
dedup'la yazar; bizde olay akisina yazilmaz, uyari anlik `alert` alaninda (kart 60sn poll).
Dogrulama: 3 yeni birim test (esik/lt, cozulemeyen->dusuk fail-closed, 45sn TTL) + canli E2E
imzali cagrida `freeBnb:"0"` + `alert:true` + ikinci cagrida `cached:true` (sifir BNB gercek
hesap durumu, uydurma degil).
UI (1.6): `BnbStatus` tipi + `apiBnb`/`useBnb` + Ayarlar'da BNB Komisyon Bekcisi karti
(DUSUK/YETERLI/ABSTAIN rozeti + onbellek notu); supurme dugmesi yok.
UI (2.6): Bilgi Merkezi'ne 3. sekme (ayri gorunum degil - mobil bar 8'de kalir). Icerik
v3'ten kopyalanmadi: durustluk 16 madde (11 GERCEK + 2 SIMULE + 3 YOK: deal-defteri, traceId
selale, ML), M01-M08 (bilinenlerde dosya adli: fraction/m02/m03_m07/m04), yol haritasi durumu
(YAPILDI 3 grup + KALAN 2.4/2.7/2.8 + tasinmazlar), docs/ 7 dosya. v3 ADR listesi uydurulmadi.
Dogrulama: tsc temiz.
OPERASYON NOTU: calisma sirasinda eski dca_server.exe (PID 416524) kilitli oldugu icin
taskkill ile durdurulup yeniden build edildi; motor su an yeni binary ile :3002'de saglikli.
DB ayni varsayilan yolda acildi (`/home/z/my-project/db/dca-core.sqlite3`); vault/botlar
etkilenmemeli - Ayarlar'da anahtarlar gorunuyorsa dogrulanmis sayilir.
Siradaki: Faz 2.7 (brackets) -> 2.4 (adli tip, sema gocu) -> 2.8 (P3 UI).

---

Task ID: faz-2-7-brackets (2026-10-22)
Ne: v3 calculator/brackets portu - venue kademe aynasi (M07 `MmrBracket`).
Motor: trait'e `mmr_brackets(symbol, creds?)` (default: desteklenmiyor) + Bybit
public `/v5/market/risk-limit` aynasi (`parse_bybit_risk_limit`: cap siralama +
cum sureklilik `cum_i = floor_i*(mmr_i-mmr_{i-1}) + cum_{i-1}`) + Binance imzali
futures `leverageBracket` aynasi (ayri `fapi` tabani: `BINANCE_FUTURES_BASE` /
testnet `BINANCE_FUTURES_TESTNET_BASE`; ayni API anahtari). Yeni uc
`GET /api/calc/brackets?venue=&symbol=` (`dca_server/src/brackets.rs`):
EXCHANGE/ABSTAIN/UNSUPPORTED(OKX/PAPER)/422(bilinmeyen venue, eksik symbol);
ASLA 500. v3 `/manual` POST'u YOK - elle giris yalniz istemci state.
maxLeverage tasinmadi (M07 tuketmiyor, notlu).
UI: Hesaplayici DCA-tabinda (linear) kademe paneli - venue secici + "Venue'den al"
+ "Elle gir" (oturum-ici tablo, kaydedilmez) + durust not (tahmin tek-kademe %0.5
kullanir, tablo katilmaz; kademe yoksa sahte uretilmez).
Dogrulama: 4 yeni kilit test (Bybit: CANLI yanittan dogrulanmis BTC ilk-3 kademe -
cap 300000/2000000/2600000, cum 510/1710; Binance: belge-sekli esleme + hata-zarfi
reddi) + canli E2E (Bybit 35 kademe EXCHANGE; Binance kasa anahtari futures-izinsiz
-> durust ABSTAIN + borsa sebebi; OKX UNSUPPORTED; 2x422) + tsc temiz.
OPERASYON NOTU: exe kilidi yuzunden motor bir kez daha taskkill+rebuild+restart
edildi (:3002 yeni binary ile saglikli).
Siradaki: Faz 2.4 (adli tip - once events'e trace_id sema gocu) -> 2.8 (P3 UI).

---

Task ID: faz-2-4-forensics (2026-10-22)
Ne: v3 events/forensics portu - tick izi + 3 modlu uc + Studio iz gorunumu.
Motor: `ALTER TABLE events ADD COLUMN trace_id TEXT` (idempotent, `migrate_columns`
deseni; eski satirlar NULL) + `AppState.tick_trace: Mutex<HashMap<bot,trace>>` +
`persist_event` haritada iz bulursa ilistirir (60 cagri yeri DEGSMEDi) +
`poll_and_advance` iz cercevesi (`tr-<bot8>-<ts>` + TICK koku `symbol/kind/venue/mode`;
cikis tek kapidan, harita sizintisi yok). Tick-disi (webhook/aksiyon) NULL kalir.
Uc: `GET /api/events/forensics` - trace (ozet: rootTs/totalMs/event/error/spanCount:0+
not + asc olaylar), traces (grup + TICK kokunden symbol/kind + minLatency iz-toplamina),
filtre (botId/errorOnly/limit; minLatency burada yok + not). ASLA 500.
UI: Studio Olaylar'da "Tick Izleri" (iz listesi + secili izin offset-barli zaman cizgisi;
span hiyerarsisi cizilmez; izsiz eski olaylar notu) + `bot_events` artik `traceId` doner.
Dogrulama: 6 yeni birim test (in-memory DB) + canli E2E (PAPER bot START->50sn->3 iz;
TICK kok + asc + bot_events traceId; 61-olaylik gercek kosu izinde kok+baglanti OK;
12/13 - tek kalan asiri-kati iddia: bos 1h tick'ine >=2 olay bekledi, davranis dogru)
+ tsc temiz. Sozlesme sekmesi guncellendi (adli tip YOK->VAR, 2.4 TODO'dan dustu).
OPERASYON NOTU: exe kilidi yuzunden bir kez daha taskkill+rebuild+restart (:3002 yeni
binary ile saglikli; DB ayni yolda acildi, vault/botlar etkilenmedi).
Siradaki: Faz 2.8 (P3 UI gruplama) - v3'ten kalan son madde.
