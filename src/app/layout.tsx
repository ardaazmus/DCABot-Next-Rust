import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";
import { Toaster } from "@/components/ui/toaster";

const inter = localFont({
  src: [
    {
      path: "./fonts/Inter-latin.woff2",
      weight: "100 900",
      style: "normal",
    },
    {
      path: "./fonts/Inter-latin-ext.woff2",
      weight: "100 900",
      style: "normal",
    },
  ],
  variable: "--font-inter",
  display: "swap",
  fallback: ["Inter", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "Roboto", "sans-serif"],
});

const jetbrainsMono = localFont({
  src: [
    {
      path: "./fonts/JetBrainsMono-latin.woff2",
      weight: "100 800",
      style: "normal",
    },
    {
      path: "./fonts/JetBrainsMono-latin-ext.woff2",
      weight: "100 800",
      style: "normal",
    },
  ],
  variable: "--font-jetbrains-mono",
  display: "swap",
  fallback: ["JetBrains Mono", "Consolas", "Courier New", "monospace"],
});

export const metadata: Metadata = {
  title: "DCABOT · Quantitative Terminal",
  description:
    "DCABOT v3.0 — 4096-bit rational arithmetic quantitative trading terminal. DCA, Grid, Infinity and Hedge strategies with 8 fail-closed risk gates.",
  icons: {
    icon: "/icon.svg",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    // Faz 22 (UIX-16 — BİLİNÇLİ KARAR): dark-first terminal. Finansal terminal
    // konvansiyonu (Bloomberg/TradingView koyu palet); palet slate/x ile koyu
    // tasarımlı olduğundan yarım bir "light mod" bilinçli olarak YOK — tema
    // istenirse ayrı bir tasarım projesi olarak yapılır.
    <html lang="tr" suppressHydrationWarning className="dark">
      <body
        className={`${inter.variable} ${jetbrainsMono.variable} antialiased bg-[#0B0E14] text-slate-200 min-h-screen flex flex-col font-sans`}
      >
        {children}
        <Toaster />
      </body>
    </html>
  );
}
