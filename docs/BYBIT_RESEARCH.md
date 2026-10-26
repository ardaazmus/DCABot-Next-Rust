# Bybit v5 API Araştırma Raporu — DCA/Grid Trading Bot Çekirdeği (Rust)

> **Araştırma tarihi:** 20 Ekim 2026 (canlı `curl` doğrulaması + resmî docs)
> **Yöntem:** `api.bybit.com` / `api-testnet.bybit.com` üzerinde canlı uç nokta çekimleri
> (gerçek JSON yanıtları aşağıda birebir kaydedildi) + resmî dokümantasyon
> (https://bybit-exchange.github.io/docs/v5/...) üzerinden alan/parametre doğrulaması.
> **Kapsam:** Algoritmik DCA/grid bot çekirdeği için gerekli REST + WebSocket yüzeyi.
> **Not:** Bu raporda yazılan tüm alan adları canlı yanıtlardan birebir kopyalanmıştır
> (yaklaşım/değer örnekleri anlık piyasa fiyatı içerir).

---

## 1. API Temelleri

### 1.1 Base URL'ler

| Ortam | REST Base URL |
|---|---|
| **Mainnet** | `https://api.bybit.com` |
| Mainnet (yedek) | `https://api.bytick.com` |
| **Testnet** | `https://api-testnet.bybit.com` |
| **Demo Trading (mainnet demo hesabı)** | `https://api-demo.bybit.com` |
| Bölgesel (TR/KZ/GE/AE/EEA/ID/JP/HK) | `api.bybit.tr`, `api.bybit.kz`, `api.bybitgeorgia.ge`, `api.bybit.ae`, `api.bybit.eu`, `api.bybit.id`, `api.manepa.jp`, `api.spark-fintech.com` |

- Canlı doğrulama: `GET https://api.bybit.com/v5/market/time` ve
  `GET https://api-testnet.bybit.com/v5/market/time` → her ikisi de `retCode: 0` döndü.
- **ABD ve Ana Çin anakara IP'leri 403 Forbidden alır** (docs: Integration Guidance).
- Yol şablonu: `{host}/{version}/{ürün}/{modül}` → örn. `api.bybit.com/v5/market/recent-trade`.

### 1.2 Yanıt zarfı (tüm REST uç noktaları)

```json
{
  "retCode": 0,          // 0 = başarı
  "retMsg": "OK",        // "OK" | "" | "success" | "SUCCESS" = başarı
  "result": { },         // iş verisi
  "retExtInfo": { },     // genelde {}
  "time": 1792475193370  // sunucu ms timestamp'i
}
```

### 1.3 Zaman / recv_window kuralları

- `X-BAPI-TIMESTAMP`: UTC ms cinsinden yerel saat (NTP senkronu önerilir).
- Geçerlilik kuralı: **`server_time - recv_window <= timestamp < server_time + 1000`**.
- `X-BAPI-RECV-WINDOW`: ms cinsinden; **varsayılan 5000**. Küçük değer daha güvenli.
- Sunucu saati: `GET /v5/market/time` → `result.timeSecond`, `result.timeNano`, üst düzey `time` (ms).
  Bot başlangıcında offset = `serverTime - localTime` hesaplanmalı.

### 1.4 Rate limit (doğrulanmış)

- **HTTP IP limiti:** 5 saniyelik pencerede **600 istek/IP** (tüm trafik).
  Aşılırsa `403, access too frequent` → **10 dk** otomatik ban; tüm HTTP oturumlarını kapat.
- **WS IP limiti:** 5 dakikada **>500 bağlantı kurulmamalı**; market data için IP başına **1000 bağlantı** üst sınırı (spot/linear/inverse ayrı sayılır).
- **UID bazlı API limiti:** saniyelik kayan pencere + UID. Aşılırsa → **`retCode: 10006`, `retMsg: "Too many visits!"`**.
- Her yanıtta limit header'ları: `X-Bapi-Limit` (limit), `X-Bapi-Limit-Status` (kalan), `X-Bapi-Limit-Reset-Timestamp` (sıfırlanma ms).

Doğrulanmış uç nokta limitleri (docs tablosu, UTA2.0):

| Yöntem | Yol | Limit |
|---|---|---|
| POST | `/v5/order/create` | 10/s (linear; Pro upgradable) |
| POST | `/v5/order/amend` | 10/s |
| POST | `/v5/order/cancel` | 10/s |
| POST | `/v5/order/cancel-all` | 10/s (linear), 1/s (inverse), 20/s (spot) |
| GET | `/v5/order/realtime` | 50/s |
| GET | `/v5/order/history` | 50/s |
| GET | `/v5/execution/list` | 50/s |
| GET | `/v5/position/list` | 50/s |
| POST | `/v5/position/set-leverage` | 10/s |
| POST | `/v5/position/switch-mode` | 10/s |
| GET | `/v5/account/wallet-balance` | 50/s |
| GET | `/v5/account/fee-rate` | 5/s |
| Batch uçları (create/amend/cancel-batch) | 1–10 emir/istek; tüketim = istek × emir sayısı | |

---

## 2. Authentication (HMAC-SWA256)

### 2.1 İmza str'si (kesin kural)

```
GET  : timestamp + api_key + recv_window + queryString      (soru işareti YOK, sıralama YOK, gönderildiği haliyle)
POST : timestamp + api_key + recv_window + jsonBodyString   (gövdeyle birebir aynı byte dizisi!)
```

- `timestamp` = ms string (header'daki `X-BAPI-TIMESTAMP` ile **aynı** olmalı),
- `recv_window` = header'daki değer ile aynı string,
- `queryString` = `category=option&symbol=...` şeklinde, `?` olmadan, **hiçbir parametre sıralaması yapılmadan** (gönderdiğin sırada),
- `jsonBodyString` = ağa yazılan JSON'un **aynı serialization'ı** (anahtar sırası/boşluk dahil). Rust'ta body'yi önce string'e serialize edip o string'i hem imzala hem gönder.
- HMAC-SHA256 ile secret üzerinden imzala → **küçük harf (lowercase) HEX** string.
  (Sistem-üretimi anahtarlar HMAC; kendin ürettiğin RSA anahtarları RSA-SHA256 + base64 kullanır — bot için HMAC yeterli.)

Resmî örnek (docs):
```
timestamp=1658384314791, api_key="XXXXXXXXXX", recv_window="5000",
queryString="category=option&symbol=BTC-29JUL22-25000-C"
signed string  = "1658384314791XXXXXXXXXX5000category=option&symbol=BTC-29JUL22-25000-C"
HMAC signature = "410e0f387bafb7afd0f1722c068515e09945610124fa11774da1da857b72f30b"
```

### 2.2 Zorunlu header'lar (private uçlar)

| Header | Değer |
|---|---|
| `X-BAPI-API-KEY` | API anahtarı |
| `X-BAPI-SIGN` | hex imza |
| `X-BAPI-TIMESTAMP` | UTC ms |
| `X-BAPI-RECV-WINDOW` | ms (vars. 5000) |
| `Content-Type: application/json` | POST için zorunlu |
| `X-Referer` / `Referer` | yalnız broker kullanıcıları |

- **`X-BAPI-SIGN-TYPE`**: Eski sürümlerde zorunluydu (değer `"2"` = HMAC-hex). Güncel resmî
  header listesinde **yer almıyor** (opsiyonel/legacy). Gönderilmesi zarar vermez; istemci basitlik için hiç göndermeyebilir.
- Zaman penceresi hatası → `retCode 10002`; imza hatası → `10004`; anahtar yoksa `POST /v5/order/create`
  canlı testte `{"retCode":10001,"retMsg":"Request parameter error: apiKey is missing"}` döndü.

---

## 3. Market Data Uç Noktaları (public, auth gerekmez)

### 3.1 `GET /v5/market/time` — sunucu saati

Canlı yanıt (20 Ekim 2026):
```json
{"retCode":0,"retMsg":"OK","result":{"timeSecond":"1792475193","timeNano":"1792475193370405943"},"retExtInfo":{},"time":1792475193370}
```

### 3.2 `GET /v5/market/instruments-info`

Parametreler: `category` (spot|linear|inverse|option, zorunlu), `symbol`, `baseCoin`, `status`, `limit` [1,1000] (vars. 500), `cursor`.

**Spot canlı yanıt** (`?category=spot&symbol=BTCUSDT`) — kritik alanlar:
```json
{"retCode":0,"retMsg":"OK","result":{"category":"spot","list":[{
  "symbolId":9,"symbol":"BTCUSDT","baseCoin":"BTC","quoteCoin":"USDT",
  "innovation":"0","status":"Trading","marginTrading":"utaOnly","stTag":"0",
  "lotSizeFilter":{
    "basePrecision":"0.000001","quotePrecision":"0.0000001",
    "minOrderQty":"0.000001","maxOrderQty":"230",
    "minOrderAmt":"5","maxOrderAmt":"8000000",
    "maxLimitOrderQty":"230","maxMarketOrderQty":"120",
    "postOnlyMaxLimitOrderSize":"1150"},
  "priceFilter":{"tickSize":"0.1"},
  "riskParameters":{"priceLimitRatioX":"0.005","priceLimitRatioY":"0.01"},
  "symbolType":"","xstockMultiplier":"1"}]},
 "retExtInfo":{},"time":1792475193735}
```
- **Spot'ta min-notional alanının güncel adı `minOrderAmt` (USDT).** Eski `minNotionalValue`
  spot lotSizeFilter'ında artık yok; docs: `minOrderQty` ve `maxOrderQty` **deprecated** —
  tip bazlı `maxLimitOrderQty` / `maxMarketOrderQty` kullanılmalı; PostOnly/RPI emirlerin limiti
  `postOnlyMaxLimitOrderSize` (limit kuantasının 5 katına kadar).

**Linear (USDT vadeli) canlı yanıt** (`?category=linear&symbol=BTCUSDT`) — kritik alanlar:
```json
{"result":{"category":"linear","list":[{
  "symbol":"BTCUSDT","contractType":"LinearPerpetual","status":"Trading",
  "baseCoin":"BTC","quoteCoin":"USDT","launchTime":"1584230400000",
  "priceScale":"2",
  "leverageFilter":{"minLeverage":"1","maxLeverage":"150.00","leverageStep":"0.01"},
  "priceFilter":{"minPrice":"0.10","maxPrice":"1999999.80","tickSize":"0.10"},
  "lotSizeFilter":{
    "maxOrderQty":"1500.000","minOrderQty":"0.001","qtyStep":"0.001",
    "postOnlyMaxOrderQty":"1500.000","maxMktOrderQty":"150.000",
    "minNotionalValue":"5"},
  "unifiedMarginTrade":true,"fundingInterval":480,"settleCoin":"USDT",
  "copyTrading":"both","upperFundingRate":"0.00333","lowerFundingRate":"-0.00333",
  "isPreListing":false,"riskParameters":{"priceLimitRatioX":"0.01","priceLimitRatioY":"0.02"},
  "forbidUplWithdrawal":false,"symbolId":5,"fullName":"Bitcoin","tags":[]}],
  "nextPageCursor":""},"retCode":0}
```
- Linear'da: `tickSize` (priceFilter), `qtyStep` + `minOrderQty`/`maxOrderQty` + `maxMktOrderQty`
  + **`minNotionalValue` (lotSizeFilter içinde!)**.
- `fundingInterval` **dakika** cinsinden (480 = 8 saat). Tickers'taki `fundingIntervalHour` saat cinsinden string.

### 3.3 `GET /v5/market/kline`

Parametreler: `category` (spot|linear|inverse; verilmezse linear), `symbol` (zorunlu),
`interval` (zorunlu): **1, 3, 5, 15, 30, 60, 120, 240, 360, 720, D, W, M**
(`D/W/M` harf olarak, tırnakla `"D"` gibi gönderilir — canlı test: `interval=7` →
`{"retCode":10001,"retMsg":"Invalid period!"}`), `start`/`end` (ms), `limit` **[1,1000]** (vars. 200).

Canlı yanıt (`?category=spot&symbol=BTCUSDT&interval=1&limit=3`):
```json
{"retCode":0,"retMsg":"OK","result":{"category":"spot","symbol":"BTCUSDT","list":[
 ["1792475160000","82010.5","82010.5","81981.8","81981.9","4.052608","332326.6835446"],
 ["1792475100000","81938.3","82010.5","81938.3","82010.5","5.071411","415709.1526257"],
 ["1792475040000","81948.6","81948.6","81915.5","81938.3","7.109319","582491.5368151"]]},
 "retExtInfo":{},"time":1792475204516}
```
- Eleman sırası: `[startTime(ms), open, high, low, close, volume, turnover]`.
- **Sıralama: `startTime`'a göre TERSTEN (yeniden eskiye)** — docs: "Sort in reverse by startTime".
- Spot: `volume` base coin (BTC), `turnover` quote coin (USDT). Linear USDT sözleşme: aynı;
  inverse'te tersine döner. `close` mum kapanmadıysa son işlem fiyatıdır.

### 3.4 `GET /v5/market/tickers`

Parametreler: `category` (zorunlu), `symbol` (option dışında opsiyonel). Linear canlı yanıtı:
```json
{"result":{"category":"linear","list":[{"symbol":"BTCUSDT",
 "lastPrice":"81944.50","indexPrice":"81981.44","markPrice":"81949.35",
 "prevPrice24h":"82667.40","price24hPcnt":"-0.008744",
 "highPrice24h":"83155.00","lowPrice24h":"81150.00","prevPrice1h":"82064.10",
 "openInterest":"59739.607","openInterestValue":"4895621962.91",
 "singleOpenInterest":"29869.804","singleOpenInterestValue":"2447811022.43",
 "turnover24h":"3680857586.9334","volume24h":"44775.8860",
 "fundingRate":"0.00005207","nextFundingTime":"1792483200000",
 "ask1Size":"10.986","bid1Price":"81944.30","ask1Price":"81944.40","bid1Size":"2.206",
 "fundingIntervalHour":"8","fundingCap":"0.00333", "...": "..." }]},"retCode":0}
```
- **Linear** için `markPrice` ve `fundingRate` buradan okunur; spot tickers: `lastPrice`,
  `ask1Price/ask1Size`, `bid1Price/bid1Size`, `prevPrice24h`, `price24hPcnt`, `highPrice24h`,
  `lowPrice24h`, `prevPrice1h`, `price24hPcnt`, `volume24h`, `turnover24h` (markPrice yok).

### 3.5 `GET /v5/market/orderbook`

Parametreler: `category`, `symbol`, `limit` (linear & spot: **1, 50, 200, 1000**; option: 25, 100).

Canlı yanıt (`?category=spot&symbol=BTCUSDT&limit=5`):
```json
{"retCode":0,"retMsg":"OK","result":{"s":"BTCUSDT",
 "a":[["81981.9","0.868013"],["81982","0.001357"]],
 "b":[["81981.8","0.558389"],["81981.7","0.006098"]],
 "ts":1792475216553,"u":62918157,"seq":115769645984,"cts":1792475216547},"retCode":0}
```
- Alanlar: `s` symbol, `a` asks [[fiyat, adet]...], `b` bids, `ts`, `u` (version), `seq`, `cts`.

### 3.6 `GET /v5/market/recent-trade`

Parametreler: `category`, `symbol`, `limit` [1,1000] (vars. 500), `baseCoin` (option).
Canlı yanıt (`?category=spot&symbol=BTCUSDT&limit=3`):
```json
{"result":{"category":"spot","list":[
 {"execId":"2290000001238778352","symbol":"BTCUSDT","price":"81981.9",
  "size":"0.000268","side":"Buy","time":"1792475211689",
  "isBlockTrade":false,"isRPITrade":false,"seq":"115769644001"}]},"retCode":0}
```

### 3.7 `GET /v5/market/funding/history` (funding geçmişi)

Parametreler: `category` (linear|inverse, zorunlu), `symbol` (zorunlu), `startTime`, `endTime` (ms), `limit` **[1,200]** (vars. 200).
Canlı yanıt:
```json
{"result":{"category":"linear","list":[
 {"symbol":"BTCUSDT","fundingRate":"0.00008881","fundingRateTimestamp":"1792454400000"},
 {"symbol":"BTCUSDT","fundingRate":"0.00005405","fundingRateTimestamp":"1792425600000"},
 {"symbol":"BTCUSDT","fundingRate":"0.00006358","fundingRateTimestamp":"1792396800000"}]},"retCode":0}
```
- Timestamp'ler 8 saat aralıklı (BTCUSDT). Mevcut/beklenen funding: tickers `fundingRate` + `nextFundingTime`.

### 3.8 `GET /v5/market/risk-limit` (MMR kademeleri — public)

Parametreler: `category` (linear|inverse), `symbol` (veya settleCoin), `cursor`.
Canlı yanıt (ilk kademeler):
```json
{"result":{"category":"linear","list":[
 {"id":1,"symbol":"BTCUSDT","riskLimitValue":"300000","maintenanceMargin":"0.0033",
  "initialMargin":"0.0066","isLowestRisk":1,"maxLeverage":"150.00","mmDeduction":""},
 {"id":2,"symbol":"BTCUSDT","riskLimitValue":"2000000","maintenanceMargin":"0.005",
  "initialMargin":"0.01","isLowestRisk":0,"maxLeverage":"100.00","mmDeduction":"510"},
 {"id":3,"symbol":"BTCUSDT","riskLimitValue":"2600000","maintenanceMargin":"0.0056",
  "initialMargin":"0.0111","isLowestRisk":0,"maxLeverage":"90.00","mmDeduction":"1710"}]},"retCode":0}
```
- `maintenanceMargin` = **MMR** (oran), `initialMargin` = IMR, `riskLimitValue` = kademe sınırı (USD notional),
  `mmDeduction` = bakım marjı düşülecek sabit tutar (USD). **`GET /v5/market/leverage-info` YOK** (404 — doğru uç `market/risk-limit`).

Diğer kullanışlı public uçlar: `/v5/market/mark-price-kline`, `/v5/market/index-price-kline`,
`/v5/market/premium-index-price-kline`, `/v5/market/open-interest`, `/v5/market/fee-group-info`
(`productType=contract` zorunlu), `/v5/market/instrument-summary`.

---

## 4. Account / Trading Uç Noktaları (private)

### 4.1 `GET /v5/account/wallet-balance`

Parametreler: `accountType` (zorunlu; **`UNIFIED`** birleşik hesap, `CONTRACT`, `SPOT`),
`coin` (virgülle çoklu: `USDT,USDC`).

Yanıt `result.list[]`: `accountType`, `accountIMRate`, `accountMMRate`, `totalEquity`,
`totalWalletBalance`, `totalMarginBalance`, `totalAvailableBalance`, `totalPerpUPL`,
`totalInitialMargin`, `totalMaintenanceMargin` ve `coin[]`:
`coin`, `equity`, `usdValue`, `walletBalance`, `locked` (spot açık emir kilidi),
`spotHedgingQty`, `borrowAmount`, `accruedInterest`, `totalOrderIM`, `totalPositionIM`,
`totalPositionMM`, `unrealisedPnl`, `cumRealisedPnl`, `bonus`, `marginCollateral`.

> **DİKKAT:** Eski `availableToWithdraw` alanı güncel şemada yok; çekilebilir bakiye için
> `/v5/asset/...` uçları veya `totalAvailableBalance` kullanılmalı. Bot DCA/grid için
> `coin[].walletBalance`, `locked`, `unrealisedPnl`, `equity` ve hesap geneli `totalAvailableBalance` yeterli.

### 4.2 `GET /v5/user/query-api` — API anahtarı izin kontrolü ⚠️ KRİTİK DÜZELTME

- Görevde geçen `query-api-key` yolu **kaldırılmış** (canlı: HTTP 404). Güncel doğru yol:
  **`GET /v5/user/query-api`** (docs sayfası: `user/apikey-info`; canlı doğrulandı: bozuk key ile
  `{"retCode":10003,"retMsg":"API key is invalid."}` döndü → uç nokta aktif).
- **Her izin seviyesindeki anahtar bu ucu çağırabilir** ("Any permission can access this endpoint").

Yanıt alanları (docs birebir):
```json
{"result":{
  "id":"2208369","note":"testnet","apiKey":"XXXXXXXX","readOnly":1,"secret":"",
  "permissions":{
    "ContractTrade":["Order","Position"],
    "Spot":["SpotTrade"],
    "Wallet":["AccountTransfer","SubMemberTransfer"],   // Withdraw için bkz. aşağıda
    "Options":[],"Derivatives":["DerivativesTrade"],
    "Exchange":["ExchangeHistory"],"Earn":["Earn"],
    "FiatP2P":["FiatP2POrder","Advertising"],"FiatBitPay":["FaitPayOrder"],
    "BitCard":["BitCard"],"ByXPost":["ByXPost"],"NFT":[],"CopyTrading":[],"BlockTrade":[],"Affiliate":[]},
  "ips":["18.181.170.164","13.212.45.47"],"type":1,
  "deadlineDay":-2,"expiredAt":"1970-01-01T00:00:00Z","createdAt":"2025-10-13T03:20:45Z",
  "uta":1,"unified":0,"userID":1448939,"vipLevel":"PRO-1",
  "rsaPublicKey":"","isMaster":true,"parentUid":"0","kycLevel":"LEVEL_1","kycRegion":"MYS"},
 "retCode":0}
```
- `permissions.Wallet` dizisi **master hesapta** çekim izni açıkken **`"Withdraw"`** değerini içerir
  (docs yorumu: `Wallet → AccountTransfer, SubMemberTransfer (master), Withdraw (master account)`).
- `readOnly`: `0` = read+write, `1` = salt okunur.
- **Sıfır-çekim güvenlik kapısı (zero-withdrawal gate) algoritması:**
  1. `readOnly == 0` değilse → bot başlamaz.
  2. `permissions.Wallet` içinde **`"Withdraw"` YOK** olmalı → çekim kapalı.
  3. `permissions.Spot` içinde `"SpotTrade"` **VAR** olmalı (spot DCA) ve/veya
     `permissions.ContractTrade` içinde `"Order"` (linear grid).
  4. `ips` boş olmamalı (IP kısıtlı anahtar tercih edilir) ve `expiredAt`/`deadlineDay` kontrol edilmeli.
  5. `uta == 1` ise hesap Unified (accountType=UNIFIED kullanılır).

### 4.3 `POST /v5/order/create` (docs sayfası: "Place Order")

Parametreler (docs'tan birebir; `false` = opsiyonel):
| Param | Zorunlu | Açıklama |
|---|---|---|
| `category` | true | `linear`, `inverse`, `spot`, `option` |
| `symbol` | true | `BTCUSDT` (yalnız büyük harf) |
| `isLeverage` | false | **yalnız spot**: `0` (vars.) normal, `1` margin/borrow (margin trading açık olmalı) |
| `side` | true | `Buy`, `Sell` |
| `orderType` | true | `Market`, `Limit` |
| `qty` | true | Spot Market **Buy varsayılan olarak değere (USDT) göre**! Perp/future/option: daima adet |
| `marketUnit` | false | Spot market emir kuantum birimi: `baseCoin` \| `quoteCoin` (BTC/USDT) |
| `price` | false | Market emirde yok sayılır; tickSize'a uy |
| `timeInForce` | false | `GTC` (vars.), `IOC`, `FOK`, `PostOnly`; Market daima IOC |
| `positionIdx` | false | **0** = one-way mode, **1** = hedge Buy tarafı, **2** = hedge Sell tarafı; hedge modda zorunlu |
| `orderLinkId` | false | Kullanıcı ID'si, maks. 36 karakter (harf/rakam/`-`/`_`) |
| `reduceOnly` | false | true → pozisyon yalnız küçülür (linear/inverse/option) |
| `closeOnTrigger` | false | Kapanış emri; tetiklenince benzer sözleşme emirlerini iptal edebilir |
| `triggerPrice` / `triggerBy` / `triggerDirection` | false | Koşullu emir parametreleri |
| `takeProfit`, `stopLoss`, `tpslMode`, `tpOrderType`, `slOrderType`, `tpLimitPrice`, `slLimitPrice`, `tpTriggerBy`, `slTriggerBy` | false | TP/SL parametreleri |
| `slippageToleranceType`/`slippageTolerance` | false | Market emir için `TickSize` / `Percent` |
| `smpType`, `mmp`, `rpiTakerAccess`, `orderFilter` | false | İleri düzey |

Yanıt: `{"result":{"orderId":"...","orderLinkId":"..."},"retCode":0}` — **asenkron**; nihai durum
WebSocket'ten doğrulanmalı.

Örnek (spot PostOnly DCA limit alım):
```json
{"category":"spot","symbol":"BTCUSDT","side":"Buy","orderType":"Limit",
 "qty":"0.001","price":"80000","timeInForce":"PostOnly","orderLinkId":"dca-0001","isLeverage":0}
```
Örnek (linear grid satışı, one-way):
```json
{"category":"linear","symbol":"BTCUSDT","side":"Sell","orderType":"Limit",
 "qty":"0.01","price":"82500","timeInForce":"PostOnly","positionIdx":0,"reduceOnly":false}
```

### 4.4 `POST /v5/order/cancel` ve `POST /v5/order/cancel-all`

- `cancel`: `category` (zorunlu), `symbol` (zorunlu), `orderId` **veya** `orderLinkId` (biri zorunlu), `orderFilter` (spot: `Order`/`tpslOrder`/`StopOrder`).
- `cancel-all`: `category` zorunlu; linear/inverse'te `symbol` **veya** `baseCoin` **veya** `settleCoin` zorunlu; `orderFilter` (spot: `Order`,`tpslOrder`,`StopOrder`,`OcoOrder`,`BidirectionalTpslOrder`; linear: `Order`,`StopOrder`,`OpenOrder`).
- İkisi de asenkron; WS `order` topic'i ile doğrula.

### 4.5 `GET /v5/order/realtime` (açık emirler) ve `GET /v5/order/history`

- `realtime`: `category` zorunlu; linear'da `symbol`/`baseCoin`/`settleCoin` gerekli; `openOnly=1` → kapanmış son 500 kayıt; `orderId`/`orderLinkId` filtreleri; limit 50/s.
- `history`: `category` zorunlu; `startTime`/`endTime` (ikisi yoksa 7 gün; fark ≤ 7 gün), `limit`, `cursor`, `orderStatus`, `orderFilter`, `orderId`/`orderLinkId`. (2 yıla kadar geçmiş.)

Emir durumları (`orderStatus`): `New`, `PartiallyFilled`, `Filled`, `Cancelled`, `Rejected`, `Untriggered`, `Triggered`, `Deactivated` (spot/lin ayrıntıları docs `order/open-order`).

### 4.6 `GET /v5/execution/list` (işlem fills)

Parametreler: `category` zorunlu; `symbol`, `orderId`, `orderLinkId`, `baseCoin`, `settleCoin`, `startTime`, `endTime` (vars. 7 gün, aralık ≤ 7 gün), `execType`, `limit` **[1,100]** (vars. 50), `cursor`.
Yanıt alanları: `symbol`, `orderId`, `orderLinkId`, `side`, `orderPrice`, `orderQty`, `leavesQty`, `orderType`, `stopOrderType`, `execFee`, `execId`, `execPrice`, `execQty`, `execType`, `execValue`, `execTime`, `feeRate`, `closedSize`, `seq`, `markPrice`, `isMaker`...

### 4.7 `GET /v5/position/list`

Parametreler: `category` (linear|inverse|option) zorunlu; linear'da `symbol` **veya** `settleCoin` zorunlu; `baseCoin`, `limit` [1,200] (vars. 20), `cursor`.
Yanıt `list[]` kritik alanlar: `positionIdx` (0/1/2), `riskId`, `riskLimitValue`, `symbol`,
`side` (`Buy`=long, `Sell`=short, boş="" pozisyon yok), `size` (daima pozitif), `avgPrice`,
`positionValue`, `autoAddMargin`, `positionStatus` (`Normal`,`Liq`,`Adl`), `leverage`,
`breakEvenPrice`, **`markPrice`**, **`liqPrice`**, **`positionIM`**, `positionIMByMp`,
**`positionMM`**, `positionMMByMp`, `takeProfit`, `stopLoss`, `trailingStop`, `unrealisedPnl`,
`cumRealisedPnl`, `createdTime`, `updatedTime`.

### 4.8 Pozisyon ayarları

- `POST /v5/position/set-leverage`: `category`, `symbol`, `buyLeverage`, `sellLeverage` (string; one-way'de ikisi eşit olmalı; hedge+isolated'ta farklı olabilir).
- `POST /v5/position/switch-mode` (hedge ↔ one-way): `category`, `symbol` veya `coin`, `mode`: **0 = Merged Single (one-way)**, **3 = Both Sides (hedge)**. Pozisyon varken `110024` hatası.
- `POST /v5/position/switch-isolated`: uç var (canlı probe: parametre hatası döndü → aktif). UTA2.0'da marj modu tercihen `POST /v5/account/set-margin-mode` ile yönetilir.
- `POST /v5/position/trading-stop` (TP/SL/trailing), `POST /v5/position/add-margin`, `POST /v5/position/set-auto-add-margin`.

### 4.9 `POST /v5/order/amend`

Parametreler: `category`, `symbol` zorunlu; `orderId` veya `orderLinkId` (biri zorunlu);
`qty` (değişmiyorsa gönderme), `price` (değişmiorsa gönderme), `triggerPrice`, TP/SL alanları.
Aynı anda hem fiyat hem miktar güncellenebilir; `40004` = emir amend sırasında değişmiş.

### 4.10 Risk limiti / MMR set etme

- Public bracket okuma: `GET /v5/market/risk-limit` (3.8'e bak).
- Pozisyondaki riskId + mmDeduction hesabı: `position/list` → `riskId`, `riskLimitValue`.
  (Docs'ta set-risk-limit için `position/set-risk-limit` uç yapısı mevcuttur; bot için bracket
  değiştirmek şart değil — varsayılan en düşük kademe 150x'e kadar yeterli.)

---

## 5. WebSocket (v5)

### 5.1 URL'ler

| Kanal | Mainnet | Testnet |
|---|---|---|
| Public Spot | `wss://stream.bybit.com/v5/public/spot` | `wss://stream-testnet.bybit.com/v5/public/spot` |
| Public Linear (USDT/USDC perp + futures) | `wss://stream.bybit.com/v5/public/linear` | `wss://stream-testnet.bybit.com/v5/public/linear` |
| Public Inverse | `wss://stream.bybit.com/v5/public/inverse` | — |
| **Private** | `wss://stream.bybit.com/v5/private` | `wss://stream-testnet.bybit.com/v5/private` |
| Order Entry (WS trade) | `wss://stream.bybit.com/v5/trade` | `wss://stream-testnet.bybit.com/v5/trade` |
| Demo (yalnız private) | `wss://stream-demo.bybit.com` (public = mainnet ile aynı) | — |
| Sistem durumu | `wss://stream.bybit.com/v5/public/misc/status` | — |

- Private/order-entry bağlantısı `?max_active_time=30s..600s` ile canlı süre ayarlanabilir; ping yoksa 10 dk'da kopar.

### 5.2 Private kimlik doğrulama mesajı

```json
{"req_id":"10001",            // opsiyonel
 "op":"auth",
 "args":["<api_key>", 1662350400000, "<signature>"]}
```
- `expires` = şimdiki zaman + 1 sn gibi ileri bir ms timestamp'i.
- **İmza: `HMAC_SHA256(secret, "GET/realtime" + expires)`** → hex (dikkat: `GET/realtime` ile sayı arasında boşluk **yok**; python örneğinde f-string `f"GET/realtime{expires}"` — boşluk yok).
- Başarı yanıtı: `{"success":true,"ret_msg":"","op":"auth","conn_id":"...-p"}`.

### 5.3 Heartbeat (ping)

- **Her ~20 saniyede bir** gönder:
```json
{"req_id":"100001","op":"ping"}
```
- Public spot/linear pong: `{"success":true,"ret_msg":"pong","op":"ping","conn_id":"..."}`;
  private pong: `{"req_id":"test","op":"pong","args":["1675418560633"],"conn_id":"..."}`.

### 5.4 Subscribe / unsubscribe

```json
{"op":"subscribe","args":["orderbook.50.BTCUSDT","publicTrade.BTCUSDT","kline.1.BTCUSDT"]}
{"op":"unsubscribe","args":["publicTrade.ETHUSDT"]}
```
- Spot public bağlantıda tek istekte en fazla **10 arg**; tüm kanallarda args toplam uzunluğu ≤ 21.000 karakter.

### 5.5 Public topic'ler

| Topic | Not |
|---|---|
| `orderbook.{depth}.{symbol}` | depth: **1, 50, 200, 1000** (spot & linear). Push: L1 10ms, L50 20ms, L200 100ms, L1000 200ms. İlk mesaj `snapshot`, sonrası `delta`. Delta kuralları: adet 0 → sil; yoksa ekle; varsa güncelle. L1'de 3 sn değişim yoksa `u` aynı snapshot yeniden gelir. |
| `publicTrade.{symbol}` | Gerçek zamanlı. Tek mesajda ≤1024 işlem. Alanlar: `T` (fill ms), `s` (symbol), `S` (taker side Buy/Sell), `v` (size), `p` (price), `i` (trade id), `BT` (block trade), `RPI`, `seq`, `L` (price direction). |
| `kline.{interval}.{symbol}` | interval: `1 3 5 15 30 60 120 240 360 720 D W M`. Push 1–60sn. Alanlar: `start`, `end`, `interval`, `open`, `close`, `high`, `low`, `volume`, `turnover`, `confirm` (mum kapandı mı), `timestamp`. |
| `tickers.{symbol}` | Push: deriv 100ms / spot 50ms. Linear alanları: `lastPrice`, `markPrice`, `indexPrice`, `prevPrice24h`, `price24hPcnt`, `highPrice24h`, `lowPrice24h`, `prevPrice1h`, `openInterest`, `openInterestValue`, `turnover24h`, `volume24h`, `nextFundingTime`, `fundingRate`, `bid1Price`, `bid1Size`, `ask1Price`, `ask1Size`, `tickDirection`, `fundingIntervalHour`. Spot alanları: `lastPrice`, `highPrice24h`, `lowPrice24h`, `prevPrice24h`, `volume24h`, `turnover24h`, `price24hPcnt`, `bid1Price/Size`, `ask1Price/Size`. |

### 5.6 Private topic'ler

| Topic | İçerik |
|---|---|
| `order` (veya kategorik `order.spot`, `order.linear`, `order.inverse`, `order.option`) | Emir durumu: `orderId`, `orderLinkId`, `symbol`, `side`, `price`, `qty`, `positionIdx`, `orderStatus`, `avgPrice`, `leavesQty`, `cumExecQty`, `cumExecValue`, `cumExecFee`, `rejectReason`, `cancelType`, `createType`, `isLeverage`. Not: cancel ile fill çakışınca **iki** `Filled` mesajı gelebilir (biri `rejectReason=EC_OrigClOrdIDDoesNotExist`). |
| `execution` | Fill bildirimi: `execId`, `orderId`, `orderLinkId`, `execPrice`, `execQty`, `execFee`, `execType`, `execPnl`, `isMaker`, `side`, `symbol`, `category`. |
| `position` | `symbol`, `side`, `size`, `avgPrice`, `positionValue`, `unrealisedPnl`, `curRealisedPnl`, `leverage`, `positionBalance`, `markPrice`, `positionIM`, `positionMM`, `tpslType`, `adlRiskLevelRatio`... |
| `wallet` | Bakiye güncellemeleri (coin bazlı `walletBalance`, `equity`, `totalEquity`, ...). |

### 5.7 Mesaj zarfı

```json
{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1792475216553,"data":[ ... ]}
{"topic":"orderbook.50.BTCUSDT","type":"delta","ts":...,"data":{...}}
```
- `type`: `snapshot` | `delta`; `ts` ms. Not (docs): Spot (tüm seviyeler) ve Futures L1'de `ts`
  alanı JSON'da `type`'tan **önce** görünebilir; parse ederken sıraya güvenme, anahtar adına göre oku.

---

## 6. Spot Ticaret Detayları

1. **`isLeverage`** (0/1): 0 = normal spot (vars.), 1 = margin borrow. Margin açıksa teminat coin'i
   tanımlı olmalı; aksi halde emir reddedilir (`170036`: Cross Margin Trading etkin değil).
2. **`marketUnit`** (YENİ): Spot **Market** emirlerde `qty` birimi:
   - Varsayılan: Market **Buy = quoteCoin değeri** (örn. BTCUSDT alımında `qty` = USDT tutarı!),
     Market **Sell = baseCoin** adedi.
   - `marketUnit:"baseCoin"` ile alımı da adet bazlı yapabilirsin. **DCA botu için kritik:**
     bütçe bazlı DCA kullanıyorsan `side:Buy, orderType:Market, qty:<USDT tutarı>` varsayılanla
     çalışır; adet bazlı istiyorsan `marketUnit:"baseCoin"` ver.
   - Docs'taki eski ifade "market buy orders are not supported for quote quantity" artık geçerli değil —
     canlı davranış: buy by value.
3. **minNotional**: spot için `lotSizeFilter.minOrderAmt` (BTCUSDT: 5 USDT). Aşağısında
   `170140` "Order value exceeded lower limit". Limit emirde `qty` daima **base coin** (BTC).
4. **Hassasiyet**: `basePrecision` (0.000001) / `quotePrecision`; fiyat için `priceFilter.tickSize` (0.1).
   Miktar adımları: qty'yi basePrecision'a göre kırp (floor).
5. **Emir tipleri**: `Limit` + `GTC/IOC/FOK/PostOnly`, `Market` (daima IOC), koşullu
   (`triggerPrice` + `orderFilter:StopOrder`), TP/SL (`takeProfit/stopLoss`, `tpslMode`), OCO (`OcoOrder`).
6. **Fee para birimi (spot)**: taker/maker ücreti işlemi gerçekleştirilen varlıktan kesilir —
   **Limit/Market Sell → ücret quote coin'den (USDT); Limit Buy → base coin'den (BTC);
   Market Buy (değer bazlı) → quote coin'den**. Bot'ta bakiye projeksiyonunda kuruş kuruş hesaba kat.
7. `orderFilter` ile spot'ta `Order` / `tpslOrder` / `StopOrder` / `OcoOrder` ayrımı yapılır;
   `cancel-all` bunlara göre filtrelenir.
8. Spot hata kodları `17xxxx` aralığıdır (bkz. Bölüm 10).

---

## 7. Ücretler ve Funding

### 7.1 Fee rate uçları

- **Kişisel ücret**: `GET /v5/account/fee-rate` → `category` (spot/linear/inverse/option),
  `symbol`; yanıt: `takerFeeRate`, `makerFeeRate`, `rpiMakerFeeRate`. Limit 5/s.
- **Public ücret grupları**: `GET /v5/market/fee-group-info?productType=contract`
  (canlı çekildi; `groupId`: 1,5,6,7,9). Canlı örneği (BTCUSDT "Major Coins" grubunda):
  Pro-1 taker 0.00028 / maker 0.0001; Pro-2 0.00025/0.00005; ... Pro-6 taker 0.00015 / maker 0.
- **Standart (Pro-0 / normal) seviye varsayılanları** (Bybit ücret sayfası, VIP olmayan kullanıcı):
  - **Spot: maker %0.10 / taker %0.10** (0.001 / 0.001)
  - **USDT Perpetual (linear): maker %0.02 / taker %0.055** (0.0002 / 0.00055)
  - Grid/DCA botu için maker'aPreferred (PostOnly) emirler maliyeti ~%55 azaltır (linear'da).

### 7.2 Funding (perp)

- **Formül**: `funding ücreti = pozisyon değeri × funding rate`
  (pozisyon değeri = `markPrice × size`). Ödeme yönü: **pozitif rate → long'lar short'lara öder**;
  negatif rate → tersi. Pozisyon değeri pozitifken long için ödeme negatif işaretli gelir
  (canlı örnek: BTCUSDT rate `0.00005207`, sonraki funding ts `1792483200000`).
- **Aralık**: varsayılan **8 saat** (`fundingInterval: 480` dk — BTCUSDT canlı doğrulandı;
  `fundingIntervalHour: "8"`). Baz pariteler 1h/4h → her sembol için instruments-info'dan
  `fundingInterval` okunmalı. Üst/alt sınır: `upperFundingRate`/`lowerFundingRate` (BTCUSDT ±0.00333; tickers'ta `fundingCap`).
- Geçmiş: `GET /v5/market/funding/history` (3.7).
- Funding zamanı hesaplanan `nextFundingTime`'a yaklaşırken grid botu emir yerleştirmeyi
  erteleyebilir; pozisyon tutarken funding maliyetini PnL'e ekle.

---

## 8. Linear (USDT Perp) Likidasyon Matematiği

### 8.1 Bakım marjı (MM)

```
MM = Σ(pozisyon değeri × MMR) − mmDeduction
```
- MMR ve mmDeduction, pozisyon notional'ının içinde bulunduğu `risk-limit` kademesinden
  (Bölüm 3.8 canlı tablo) alınır: BTCUSDT kademe-1: MMR 0.33%, mmDeduction 0;
  kademe-2 (2M USD'ye kadar): MMR 0.5%, deduction 510 USD; ...
- IMR (başlangıç marj oranı) = 1 / kaldıraç; `initialMargin` alanı = kademe IMR'si.

### 8.2 Likidasyon tetik koşulu (resmî denklem)

Bybit likidasyon protokolü: **`W + uPnL ≤ MM`** anında pozisyon likide edilir.
- `W` (Wallet/Margin Balance) = izole modda pozisyona ayrılan bakiye; cross/UTA'da hesap teminatı.
- `uPnL` = işaretlenmemiş kar/zarar (markPrice üzerinden).
- Yani: `MarginBalance = W + uPnL`, `MM ≥ MarginBalance` → auto-liquidation.

### 8.3 Tahmini liqPrice (izole, linear, yaklaşımlar hariç)

```
Long : liqPrice ≈ avgEntryPrice × (1 − IMR + MMR)
Short: liqPrice ≈ avgEntryPrice × (1 + IMR − MMR)
```
- Fonlama ödemeleri, işlem ücretleri ve `mmDeduction` sabit terimi biraz kaydırır
  (mmDeduction dahil hâli: Long: `liqPrice ≈ (entry×qty − W + MM) / qty`).
- **Cross/UTA'da liqPrice yalnızca sunucu tarafında tahmin edilir** — bot daima
  `/v5/position/list` → `liqPrice` alanını authoritative kabul etmeli; alan
  `liqPrice <= minPrice` veya `>= maxPrice` olduğunda `""` döner.
- `positionStatus: "Liq"` pozisyon likidasyon sürecine girdiğini gösterir; ayrıca
  `Adl` = ADL riski. Güvenlik: bot `positionMM`'i `positionIM` + uPnL ile sürekli izler
  ve MM/bakiye oranı eşiği (örn. %80) aşılırsa pozisyonu kendisi kapatır.

---

## 9. Testnet ve Demo

- **Testnet REST**: `https://api-testnet.bybit.com` (canlı doğrulandı — `/v5/market/time` 200).
- **Testnet WS**: `wss://stream-testnet.bybit.com/v5/public/{spot|linear|inverse}`,
  `wss://stream-testnet.bybit.com/v5/private`, `wss://stream-testnet.bybit.com/v5/trade`.
- **Anahtar üretimi**: https://testnet.bybit.com adresinde hesap aç → Avatar → API →
  sistem üretimli key/secret (HMAC). Testnet anahtarları **yalnız** testnet host'unda geçerli
  (yanlış ortamda `10003`: "Check whether the key and domain are matched, there are 4 env:
  mainnet, testnet, mainnet-demo, testnet-demo").
- **Demo trading** (mainnet simülasyonu): `https://api-demo.bybit.com` + WS
  `wss://stream-demo.bybit.com` (yalnız private; public veri mainnet'le aynı). Anahtarları
  mainnet hesabında Demo Trading moduna geçip üret. Demo emirleri 7 gün saklanır, limit yükseltilemez.
- Testnet'te gerçek bakiye fonlamak gerekmez; UI üzerinden faucet/claim kullanılır.

---

## 10. Hata Kodları

Zarf: `retCode != 0` → hata. **Canlı doğrulanmışlar**: `10001` (parametre hatası; ayrıca
`"Invalid period!"`, `"Request parameter error: apiKey is missing"` varyantları),
`10003` ("API key is invalid."), `10002` (zaman penceresi), `10004` (imza), `10005` (izin),
`10006` (rate limit "Too many visits!"), `10010` (IP eşleşmedi), `429` (yük), `110007`,
`110017`, `170131`.

| retCode | Anlamı |
|---|---|
| 0 | Başarı |
| 10001 | Request parameter error (eksik/geçersiz param; `"apiKey is missing"`, `"Invalid period!"` ...) |
| 10002 | Zaman penceresi dışı timestamp |
| 10003 | API key geçersiz / ortam uyuşmuyor |
| **10004** | **İmza hatası** — imza algoritmasını kontrol et |
| 10005 | İzin yok (API key permissions) |
| **10006** | Rate limit ("Too many visits!") |
| 10010 | IP, anahtara bağlı IP listesinde değil |
| 10014 | Duplicate request (aynı istek tekrarı) |
| 10016 | Sunucu hatası / restart |
| 10028 | Bu uç yalnız Unified hesaplara açık |
| 10029 | Geçersiz sembol |
| 110001 | Emir bulunamadı |
| 110003 | Emir fiyatı aralık dışı |
| 110004 | Wallet balance insufficient |
| **110007** | **Available balance insufficient** (marj/kullanılabilir bakiye yetersiz) |
| 110008/110010 | Emir dolmuş/iptal edilmiş |
| 110011 | Bu ayar likidasyonu anında tetikler |
| 110016 | Risk limit kademesini aşıyor |
| **110017** | **orderQty sıfıra yuvarlanacak** (qty/precision hatası) |
| 110020 | 500 aktif emir sınırı |
| 110024 | Pozisyon varken position mode değiştirilemez |
| 110025/110026 | Mode / marj modu zaten istenen |
| 170115/116/117/121 | Spot: geçersiz timeInForce / orderType / side / symbol |
| 170130 | Parametre değeri geçersiz |
| **170131** | **Spot: Balance insufficient** |
| 170132/170133 | Fiyat üst/alt sınırları |
| **170140** | **Order value lower limit altında (spot minNotional!)** |
| 170134/170137/170148 | Fiyat/adet ondalık uzunluk |
| 170136/170381 | Adet alt/üst sınır |
| 170139/170142/170143 | Emir dolmuş / iptal edilmiş / orderbook'ta yok |
| 170151/170157 | Parite kapalı / API ticaretine kapalı |

---

## 11. Rust Uygulama Notları

### 11.1 İmza (pseudocode)

```rust
// HTTP GET
fn sign_get(secret: &str, ts: u64, recv_window: &str, api_key: &str, query: &str) -> String {
    let payload = format!("{ts}{api_key}{recv_window}{query}"); // query '?'siz, gönderim sırasıyla
    hex_lower(hmac_sha256(secret.as_bytes(), payload.as_bytes()))
}
// HTTP POST — body STRİNİNİN kendisini imzala, sonra aynı string'i gönder
fn sign_post(secret: &str, ts: u64, recv_window: &str, api_key: &str, body: &str) -> String {
    let payload = format!("{ts}{api_key}{recv_window}{body}");
    hex_lower(hmac_sha256(secret.as_bytes(), payload.as_bytes()))
}
// WS private auth
let expires = now_ms() + 1_000;
let sig = hex_lower(hmac_sha256(secret, format!("GET/realtime{expires}"))); // boşluk YOK
// send {"op":"auth","args":[api_key, expires, sig]}
```
Header seti: `X-BAPI-API-KEY, X-BAPI-SIGN, X-BAPI-TIMESTAMP, X-BAPI-RECV-WINDOW,
Content-Type: application/json` (+ opsiyonel legacy `X-BAPI-SIGN-TYPE: 2`).
`serde_json::to_string` çıktısını her zaman aynı instance'ta imzala ve gönder; anahtar
sırası deterministik olsun (BTreeMap veya struct alan sırası).

### 11.2 Kütüphane önerileri

- HTTP: `reqwest` (rustls) + `tokio`; JSON: `serde_json`; Decimal: **`rust_decimal`** (f64 YASAK —
  qty/price/fee hesapları için). tickSize/qtyStep kırpma: `Decimal::floor()`.
- HMAC: `hmac` + `sha2`; hex: `hex` crate.
- WS: `tokio-tungstenite` (rustls). Reconnect: exponential backoff + snapshot reset.
- Resmî Rust SDK da mevcut (bybit-exchange/bybit-rust) — çekirdeğe değil, referans teste iyi gelir.

### 11.3 Saat senkronizasyonu

Başlangıçta ve her 10 dk'da `GET /v5/market/time` → `offset = server_ms - local_ms`;
`X-BAPI-TIMESTAMP = local_ms + offset`. recv_window 5000 ms (NTP'li sunucuda 2000-5000).

### 11.4 Rate limit yönetimi

UID bazlı token-bucket (order 10/s, query 50/s) + `X-Bapi-Limit-Status` header'ını okuyup
adaptive throttle; `10006`/`429` görülünce jitter'lı backoff; IP limiti 600/5s'i asla zorlama.

### 11.5 Bot'un ihtiyaç duyduğu uç nokta listesi (checklist)

**Başlangıç / güvenlik**
- [ ] `GET /v5/market/time` — saat offseti
- [ ] `GET /v5/user/query-api` — readOnly==0, `Withdraw` izni YOK, Spot/ContractTrade izinleri VAR (zero-withdrawal gate)
- [ ] `GET /v5/market/instruments-info?category=spot|linear&symbol=...` — tickSize, qtyStep/minOrderQty, minOrderAmt/minNotionalValue, maxMktOrderQty, maxLeverage

**Veri**
- [ ] `GET /v5/market/kline` (veya WS `kline.{interval}.{symbol}`) — sinyal
- [ ] WS `orderbook.{50|200}.{symbol}` snapshot+delta bakımı
- [ ] WS `publicTrade.{symbol}`, `tickers.{symbol}` (markPrice/fundingRate)
- [ ] `GET /v5/market/funding/history` — funding rejimi analizi

**Bakiye / pozisyon**
- [ ] `GET /v5/account/wallet-balance?accountType=UNIFIED` (spot: `SPOT`)
- [ ] `GET /v5/position/list?category=linear&symbol=...` — liqPrice/MM izleme
- [ ] `GET /v5/account/fee-rate` — maliyet modeli

**Emir yaşam döngüsü**
- [ ] `POST /v5/order/create` (PostOnly grid; spot DCA için market+marketUnit ya da limit)
- [ ] `POST /v5/order/cancel`, `POST /v5/order/cancel-all`
- [ ] `POST /v5/order/amend` (grid kaydırma)
- [ ] `GET /v5/order/realtime` + WS `order` topic (autoritatif durum)
- [ ] `GET /v5/execution/list` + WS `execution` (fill muhasebesi)
- [ ] `GET /v5/order/history` — barındırma/rökonciliasyon

**Pozisyon kontrolü (linear grid)**
- [ ] `POST /v5/position/set-leverage`
- [ ] `POST /v5/position/switch-mode` (mode 0 one-way; 3 hedge)
- [ ] `GET /v5/market/risk-limit` — MMR bracket + mmDeduction
- [ ] (ops.) `POST /v5/position/switch-isolated` veya `POST /v5/account/set-margin-mode`

**WS yaşam döngüsü**
- [ ] Private: auth (5.2) → subscribe `order`, `execution`, `position`, `wallet` → 20 sn'de `{"op":"ping"}`
- [ ] Public: `orderbook.50`, `publicTrade`, `kline.1|60`, `tickers` → ping aynı

### 11.6 Tasarım uyarıları

1. Tüm emir mutasyonları **asenkron**: create/cancel onayı WS `order` topic'inden gelir;
   REST onayı yalnızca kabul demektir.
2. `orderLinkId` zorunlu kullan (bot-uniqueness: `{bot}-{seq}`); WS ile REST arasında bağ için şart.
3. Spot'ta Market Buy `qty`'nin varsayılan birimi **USDT** — para birimi birimini yanlış
   varsayan klasik bug'a dikkat (Bölüm 6.2).
4. `qty`/`price` her zaman string; `rust_decimal` ile üret, gereksiz sıfırları kırp.
5. Hedge mode kullanacaksan her emirde `positionIdx` gönder; one-way'de 0.
6. Linear'da `reduceOnly:true` kapanış emirlerinde pozisyon flip'ini engeller.
7. Funding saatlerinde (her 8h: 00:00/08:00/16:00 UTC ±) likidite/fiyat oynaması grid botunda
   kısa süreli askıya alma için iyi bir tetikleyicidir.
