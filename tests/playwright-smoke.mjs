// @ts-check
/**
 * Faz 28 (C) — Playwright smoke: 5 ekran altın-yol.
 *
 * Salt-okunur gezi: hiçbir yıkıcı aksiyon (KILL/Sil/Kapat/Duraklat)
 * tetiklenmez. Motor + UI ayakta olmalı: `npm run local:start`.
 * Çalıştırma: npx playwright test tests/playwright-smoke.mjs
 */
import { test, expect } from "@playwright/test";

const BASE = "http://127.0.0.1:3000";

/** React hidrasyonu için kısa bekleme — SSE kalıcı bağlantısı
 * networkidle'ı engellediğinden load + sabit gecikme kullanılır. */
async function gotoApp(page, path = "/") {
  // SSE kalıcı bağlantısı "load"/networkidle'ı engelleyebilir; DOMContentLoaded
  // + hidrasyon işareti (client-side tazelik rozeti) beklenir.
  await page.goto(BASE + path, { waitUntil: "domcontentloaded", timeout: 120_000 });
  // Kimlik sigortası: :3000'i başka bir uygulama kapmışsa (port yarışı)
  // testler yanlış siteye karşı koşmasın — net hata ver.
  await page.waitForFunction(
    () => document.querySelector("header")?.textContent?.includes("DCABOT") ?? false,
    { timeout: 30_000 },
  ).catch(() => {
    throw new Error(
      `:3000 üzerinde DCABOT yok (başka uygulama dinliyor olabilir) — npm run local:start ile yeniden başlatın`,
    );
  });
  await page
    .waitForFunction(
      () => /VERİ|VERI/.test(document.querySelector("footer")?.textContent ?? ""),
      { timeout: 60_000 },
    )
    .catch(() => {}); // rozet yoksa test beklentileri yine de çalışır
  await page.waitForSelector("text=CORE v", { timeout: 30_000 }).catch(() => {});
  await page.waitForTimeout(500);
}

test.describe.configure({ mode: "serial" });

test("kokpit: KPI kartları + bot listesi + footer durumu", async ({ page }) => {
  await gotoApp(page);
  await expect(page.getByText("TOPLAM ÖZKAYNAK", { exact: false })).toBeVisible({ timeout: 20000 });
  await expect(page.getByText(/bot filosu/i).first()).toBeVisible();
  const newsToggle = page.getByText(/Piyasa Nabzı & Kaynak Durumu/i);
  if (await newsToggle.isVisible().catch(() => false)) {
    await newsToggle.click();
  }
  await expect(page.getByText("Borsa Duyuruları")).toBeVisible();
  await expect(page.locator("footer")).toContainText("KORUMA:");
});

test("filo: arama/filtre/sıralama kontrolleri + politika şeridi + HALT kurtarma", async ({ page }) => {
  await gotoApp(page);
  await page.getByRole("button", { name: "Bot Filosu" }).click();
  await expect(page.getByText(/Bot Filosu \(/)).toBeVisible({ timeout: 20000 });
  await expect(page.getByPlaceholder("Ara: bot / sembol")).toBeVisible();
  await expect(page.getByText("Piyasa koruması — tüm filoda aynı politika:")).toBeVisible();
  // HALT'lı bot varsa kartta kurtarma düğmesi; yoksa düğme olmamalı (koşullu render)
  const haltPill = page.getByText("KERNEL HALT", { exact: true }).first();
  if (await haltPill.isVisible().catch(() => false)) {
    await expect(page.getByRole("button", { name: /KERNEL'İ SIFIRLA/ }).first()).toBeVisible();
  }
});

test("studio: K/Z birinci kademe + Deal Durumu faz-bazlı", async ({ page }) => {
  await gotoApp(page);
  await page.getByRole("button", { name: "Bot Filosu" }).click();
  await page.getByRole("button", { name: "İncele" }).first().click();
  await expect(page.getByText("K/Z (GERÇEKLEŞEN)")).toBeVisible({ timeout: 20000 });
  await expect(page.getByText("Deal Durumu")).toBeVisible();
  // UIX-20: faz-bazlı metin — "invariant" jargonu yasak
  const deal = await page.getByText(/deal/i).first().textContent();
  expect(deal || "").not.toContain("invariant");
});

test("hesaplayıcı: insan altbaşlık + ladder fiyat biçimi", async ({ page }) => {
  await gotoApp(page);
  await page.getByRole("button", { name: "Hesaplayıcı" }).click();
  await expect(page.getByText(/Sandbox hesaplayıcı/)).toBeVisible({ timeout: 20000 });
  await expect(page.getByText(/motorla birebir aynı hesap çekirdeği/)).toBeVisible();
  // UIX-31: 8 ondalık gürültü yasak — fiyatlar kısa biçimde
  const body = await page.locator("main").innerText();
  expect(body).not.toMatch(/\d{3}\.\d{6,}/);
});

test("ayarlar: YZ sağlayıcı tek panel + kasa", async ({ page }) => {
  await gotoApp(page);
  await page.getByRole("button", { name: "Ayarlar" }).click();
  await expect(page.getByText("YZ Sağlayıcısı (Özetler & Teşhis)")).toBeVisible({ timeout: 20000 });
  await expect(page.getByText("API Key Vault (AES-256-GCM)")).toBeVisible();
});
