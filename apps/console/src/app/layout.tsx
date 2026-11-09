import type { Metadata, Viewport } from "next";
import { IBM_Plex_Mono, Schibsted_Grotesk } from "next/font/google";
import { headers } from "next/headers";

import { DesktopShellMark } from "@/components/desktop-shell-mark";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import "./globals.css";
import { cn } from "@/lib/utils";
import { LocaleSync } from "@/i18n/locale-sync";
import { requestLocale, serverT } from "@/i18n/server";

/**
 * Schibsted Grotesk for the desk, IBM Plex Mono for machine text.
 *
 * Schibsted is a Norwegian newspaper grotesque — the same north-sea register
 * as Hárbarðr — and it holds 13–15px without the costume of a display face.
 * Rejected: Geist (the last desk), Inter/Manrope (2026 dashboard default),
 * a serif body (this is a radio, not a journal).
 *
 * Plex Mono carries ids, clocks, and inline code. It is tabular by default,
 * so the rails do not jitter. `next/font/google` self-hosts at build time.
 */
const schibsted = Schibsted_Grotesk({
  subsets: ["latin"],
  weight: "variable",
  variable: "--font-schibsted",
  display: "swap",
});

const plexMono = IBM_Plex_Mono({
  subsets: ["latin"],
  weight: ["400", "500", "600"],
  variable: "--font-plex-mono",
  display: "swap",
});

/**
 * The console is loopback-only and `noindex`, so this metadata is not for
 * search engines. It is what the operator's own surfaces read: the browser tab
 * and bookmark, the desktop shell's title, and the card any chat client draws
 * when the localhost URL is pasted into a thread.
 *
 * `metadataBase` has to be an absolute URL for `opengraph-image` to resolve,
 * and the only absolute URL this product has is the loopback port it serves
 * itself on.
 */
export async function generateMetadata(): Promise<Metadata> {
  const t = await serverT("shell");
  return {
    metadataBase: new URL("http://127.0.0.1:3419"),
    title: t("metaTitle"),
    description: t("metaDescription"),
    applicationName: "harbard",
    robots: { index: false, follow: false },
    openGraph: {
      type: "website",
      siteName: "harbard",
      title: t("metaTitle"),
      description: t("metaDescriptionShort"),
    },
  };
}

export const viewport: Viewport = {
  // Neutral charcoal for the night-first workspace.
  themeColor: "#181b1f",
};

// Per-request, never cached: every page renders with live data. The bus
// snapshot and its stream belong to the previous console only, in
// `(bus)/layout.tsx`; the workspace shell reads `/workspace` instead.
export const dynamic = "force-dynamic";

export default async function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // The CSP (src/proxy.ts) runs an inline script only with this request's nonce.
  const nonce = (await headers()).get("x-nonce") ?? undefined;
  const locale = await requestLocale();
  return (
    <html
      lang={locale}
      className={cn("dark", schibsted.variable, plexMono.variable, "font-sans")}
      suppressHydrationWarning
    >
      <head>
        {/* Before paint: reserve the macOS traffic-light strip when hosted in
            the Tauri shell. The client mark keeps dataset in sync after load. */}
        <script
          nonce={nonce}
          dangerouslySetInnerHTML={{
            __html: `(function(){try{var q=new URLSearchParams(location.search).get("shell")==="desktop";if(q||"__TAURI_INTERNALS__"in window||"__TAURI__"in window||/Tauri/i.test(navigator.userAgent))document.documentElement.dataset.shell="desktop"}catch(e){}})();`,
          }}
        />
      </head>
      <body className="min-h-dvh font-sans">
        {/* Mounted once, here: Base UI's per-root tooltip delay is 600ms, and
            the Provider is what lets an adjacent tooltip open instantly. The
            rail exists to be glanced across, and without this every hop cost a
            fresh 600ms, so its labels effectively never appeared. */}
        <LocaleSync locale={locale}>
          <TooltipProvider>
            <DesktopShellMark />
            {/* No container here: the chat surface is full-bleed like a client
                app, while the reading pages set their own measure. */}
            <main>{children}</main>
            <Toaster />
          </TooltipProvider>
        </LocaleSync>
      </body>
    </html>
  );
}
