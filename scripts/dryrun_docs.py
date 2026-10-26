"""One-shot: Faz 1.1 kayitlari (roadmap + envanter + worklog)."""

R_OLD = '| 1.1 | Dry-run (`v3: /api/live/dryrun`'
assert R_OLD in open("docs/YOL_HARITASI_KALAN.md", encoding="utf-8").read()

with open("docs/YOL_HARITASI_KALAN.md", encoding="utf-8") as f:
    ym = f.read()
start = ym.index(R_OLD)
end = ym.index("\n", start)
ym = ym[:start] + (
    "| 1.1 | Dry-run (`v3: /api/live/dryrun` -> `buildDryRunOrder`: M08 tick/step/minNotional hizalama, "
    "`sent:false`, anahtar olsa bile emir yok) | **TASI (P1) - YAPILDI 2026-10-22** | "
    "`POST /api/live/dryrun` (Binance spot kurallari, LIMIT SELL->tickUp/digerleri->tickDown, qty asagi, "
    "minQty/minNotional ihlalinde otomatik buyutme YOK->ok:false) + Studio Ozet'te \"0-Risk Dene\" paneli. "
    "Iki bilinclI sapma (kodda D1/D2/D3 isaretli): MARKET+quoteOrderQty dogrudan minNotional'a karsi (v3 hep ok:false verirdi); "
    "fiyatsiz MARKET'ta ticker last notional referansi (`refPrice` aciklanir); STOP'ta price yoksa stopPrice referans. "
    "Dogrulama: dca_server 17/17 (7 yeni) + workspace 229/229; canli E2E BUY->tickDown/SELL->tickUp, "
    "MARKET+quote ok:true, minQty/minNotional ret (buyutmesiz), eksik alan 422; tarayici: panel GONDERILEBILIR rozeti + hizalama satiri. |"
) + ym[end:]
with open("docs/YOL_HARITASI_KALAN.md", "w", encoding="utf-8", newline="") as f:
    f.write(ym)
print("ok: roadmap")

E_OLD = "| Dry-run (0-risk filtre dogrulama) | **YOK** | Port adayi **P3** |"
with open("docs/ENVANTER_H_V3_PORT.md", encoding="utf-8") as f:
    en = f.read()
assert E_OLD in en, "envanter anchor yok"
en = en.replace(
    E_OLD,
    "| Dry-run (0-risk filtre dogrulama) | **VAR** | `POST /api/live/dryrun` (M08, sent:false, fail-closed) "
    "+ Studio \"0-Risk Dene\" paneli; Binance `exchangeInfo` artik sembol-filtreli cekiliyor (hizli) + venue-nitelikli onbellek (Faz 1.1) |",
)
with open("docs/ENVANTER_H_V3_PORT.md", "w", encoding="utf-8", newline="") as f:
    f.write(en)
print("ok: envanter")

WL = """
---

Task ID: faz-1-1-dryrun (2026-10-22)
Ne: v3 `POST /api/live/dryrun` portu — M08 hizalama, `sent:false`, aga emir YOK.
Motor: yeni `dca_server/src/dryrun.rs` — `POST /api/live/dryrun` (Binance spot kurallari; LIMIT+SELL->tickUp,
digerleri->tickDown; miktar asagi; minQty/minNotional ihlalinde otomatik BUYUTME YOK->`ok:false`).
Ucuncu sapma haric v3 birebir; 3 bilinclI sapma kodda D1/D2/D3 isaretli: (D1) MARKET+quoteOrderQty dogrudan
minNotional'a karsi dogrulanir (v3 `quantity ?? '0'`'dan hesaplayip hep `ok:false` donerdi);
(D2) fiyatsiz MARKET'ta halka-acik ticker `last_price` notional referansi, yanitta `refPrice` diye aciklanir;
(D3) STOP_MARKET/TAKE_PROFIT_MARKET'ta `price` yoksa `stopPrice` referans alinir. Kural yoksa/upstream
hatasi->422 fail-closed (varsayilan kural YOK).
Altyapi: `dca_exchange` Binance `instruments()` artik `symbol` verilince sunucu-taraflI filtreliyor
(`exchangeInfo?symbol=X`, 5KB — tam liste MB'lerce, yavas hatta 15sn istemci zaman asimini asiriyordu);
venue-nitelikli (`BINANCE:spot:X`) instruments_cache (motorun Bybit anahtarlariyla cakismaz).
UI: Studio Ozet'te \"Dry-Run - 0-Risk Emir Provasi\" paneli (yon/tip/miktar/fiyat, bot semboluyle; GONDERILEBILIR/
GONDERILEMEZ rozeti + hizalama satirlari + order JSON + note) + `apiDryRun` + `DryRunRequest/Result` tipleri.
Dogrulama: `cargo test -p dca_server` 17/17 (7 yeni dryrun testi); workspace 229/229; tsc temiz; lint 0 error.
Canli E2E (gercek motor :3002): LIMIT BUY 67239.567->67239.56 tickDown / SELL->67239.57 tickUp (v3-parite);
MARKET+quoteOrderQty=10 -> ok:true (D1 kaniti); MARKET+qty fiyatsiz -> ok:true + refPrice notu (D2);
qty 0.000001 -> ok:false \"minQty\" (buyutmesiz, aligned 0.00000); quote 3 USDT -> ok:false minNotional;
eksik alan -> 422; `sent:false` hepsinde. Tarayici (:3101 dev + gateway): 0-Risk Dene -> GONDERILEBILIR +
\"price: 81650.567 -> 81650.56 (tickDown (M08))\" + order JSON render; console error yok.
Not: ara adimda Binance tam-liste cekimi 15sn+decode-hatasi verdi (yavas hat) — sembol-filtre + onbellek
sonrasi soguk 0.5sn / onbellekli 0.1sn. Siradaki: Faz 1.2 `/api/live/status` rollup.
"""
with open("worklog.md", "a", encoding="utf-8", newline="") as f:
    f.write(WL)
print("ok: worklog")
print("ALL DONE")
