import type { Metadata } from "next";
import type { ReactNode } from "react";
import { Inter, JetBrains_Mono } from "next/font/google";
import { cn } from "@/lib/cn";
import SiteHeader from "@/components/SiteHeader";
import SiteFooter from "@/components/SiteFooter";
import "./globals.css";

// Exposed as CSS variables rather than applied via `.className` directly:
// `--ak-font-sans`/`--ak-font-mono` in globals.css reference these variable
// names, which is what lets the token copy's own font stack — shared with
// the Astro sites, which load the same two faces a different way — prefer
// next/font's self-hosted, hashed files without ever needing to know this
// app exists.
const inter = Inter({ subsets: ["latin"], variable: "--font-inter" });
const jetbrainsMono = JetBrains_Mono({
  subsets: ["latin"],
  variable: "--font-jetbrains-mono",
});

export const metadata: Metadata = {
  title: "Authkestra Playground",
  description:
    "Toggle auth features, see a real config diff, and try the flows live.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={cn(inter.variable, jetbrainsMono.variable)}>
      <body className="min-h-screen font-sans antialiased">
        <div className="flex min-h-screen flex-col">
          <SiteHeader />
          {children}
          <SiteFooter />
        </div>
      </body>
    </html>
  );
}
