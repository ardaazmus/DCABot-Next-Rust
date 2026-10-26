import type { Metadata } from "next";
import { Inter, JetBrains_Mono } from "next/font/google";
import "./globals.css";
import { Toaster } from "@/components/ui/toaster";

const inter = Inter({
  variable: "--font-inter",
  subsets: ["latin", "latin-ext"],
  display: "swap",
});

const jetbrainsMono = JetBrains_Mono({
  variable: "--font-jetbrains-mono",
  subsets: ["latin", "latin-ext"],
  display: "swap",
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
