//! Faz 1.6 BNB komisyon bekcisi — GET only.
//!
//! v3 eslemesi: `GET /api/live/bnb` -> serbest BNB bakiyesi + esik + uyari
//! (45 sn TTL onbellek). v3 POST supurme (`dust-transfer`) TASINMAZ (R.2:
//! mainnet'te gercek varlik hareketi; testnet SAPI desteklemez).
//!
//! Kurallar:
//! - Binance anahtari yoksa ABSTAIN (`freeBnb: null`, sahte sifir YOK).
//! - Imzali cagri basarisizsa ABSTAIN + sebep (140 karakter kirpilir).
//! - Bu uc ASLA 500 donmez (Json<Value>, her bacak ayri korunur).
//! - Bilincli sapma: v3 dusuk bakiyede `BNB_LOW_ALERT` olayini 30dk
//!   dedup'la olay akisina yazar; bizde olay akisina YAZILMAZ — uyari anlik
//!   `alert` alaninda raporlanir, kart 60sn poll'lar. Olay carki kirlenmez.

use axum::extract::State;
use axum::Json;
use dca_exchange::VenueAdapter;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::state::{now_ms, AppState};

/// BNB kritik esigi (v3 `BNB_LOW_THRESHOLD` aynen).
pub const BNB_LOW_THRESHOLD: &str = "0.05";
/// Durum TTL (v3 `STATUS_TTL_MS` aynen).
pub const BNB_TTL_MS: i64 = 45_000;
/// Imzali cagri zaman asimi (ping 4sn'den genis — account agir ucdur).
const WALLET_TIMEOUT: Duration = Duration::from_secs(8);

/// Serbest bakiye esigin altinda mi (fail-closed: cozulemezse DUSUK say).
pub fn is_low(free: &str) -> bool {
    let f = dca_math::Fraction::from_any_str(free).unwrap_or_else(|_| dca_math::Fraction::zero());
    let t = dca_math::Fraction::from_any_str(BNB_LOW_THRESHOLD)
        .unwrap_or_else(|_| dca_math::Fraction::zero());
    f < t
}

/// Onbellek tazeligi (saf — test icin ayrik).
pub fn cache_fresh(at_ms: i64, now_ms: i64) -> bool {
    now_ms - at_ms < BNB_TTL_MS
}

fn exchange_body(free: &str, cached: bool) -> Value {
    json!({
        "freeBnb": free,
        "threshold": BNB_LOW_THRESHOLD,
        "alert": is_low(free),
        "source": "EXCHANGE",
        "cached": cached,
        "at": now_ms(),
        "note": "Binance spot serbest BNB (komisyon indirimi icin); dusukse komisyon base asset'ten kesilir.",
    })
}

fn abstain_body(reason: &str) -> Value {
    json!({
        "freeBnb": Value::Null,
        "threshold": BNB_LOW_THRESHOLD,
        "alert": false,
        "source": "ABSTAIN",
        "reason": reason,
        "cached": false,
        "at": now_ms(),
    })
}

pub async fn live_bnb(State(app): State<Arc<AppState>>) -> Json<Value> {
    if let Some((at, free)) = app.bnb_cache.lock().await.clone() {
        if cache_fresh(at, now_ms()) {
            return Json(exchange_body(&free, true));
        }
    }
    let creds = match crate::active_keypair(&app, "BINANCE") {
        Some(c) => c,
        None => return Json(abstain_body("BINANCE anahtari yok (abstain)")),
    };
    let res = tokio::time::timeout(WALLET_TIMEOUT, app.binance.wallet(&creds, "BNB")).await;
    match res {
        Ok(Ok(w)) => {
            let free = w.available.to_string();
            *app.bnb_cache.lock().await = Some((now_ms(), free.clone()));
            Json(exchange_body(&free, false))
        }
        Ok(Err(e)) => Json(abstain_body(&e.chars().take(140).collect::<String>())),
        Err(_) => Json(abstain_body("zaman asimi (8sn)")),
    }
}

#[cfg(test)]
mod bnb_tests {
    use super::*;

    #[test]
    fn threshold_matches_v3_point_zero_five() {
        assert!(is_low("0.049"));
        assert!(!is_low("0.05")); // esikte uyari yok (v3 `lt` aynen)
        assert!(!is_low("0.051"));
        assert!(!is_low("1.5"));
    }

    #[test]
    fn unparsable_is_low_fail_closed() {
        // Cozulemez bakiye "yuksek" gosterilmez — uyari tarafina yatar.
        assert!(is_low("bogus"));
        assert!(is_low(""));
    }

    #[test]
    fn cache_ttl_is_45s() {
        assert!(cache_fresh(1000, 1000 + 44_999));
        assert!(!cache_fresh(1000, 1000 + 45_000));
        assert!(cache_fresh(2000, 1000)); // saat carpikligi — eski degil, taze say
    }
}
