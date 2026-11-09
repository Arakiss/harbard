import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

/**
 * Nonce-based Content-Security-Policy.
 *
 * The console renders untrusted bus content, so CSP is the second wall behind
 * React's escaping (row B4): even a hypothetical injected script has no nonce
 * and cannot run, and `connect-src` pins the only reachable origin to the local
 * store API — nothing can exfiltrate. A per-request nonce keeps `script-src`
 * strict (no `'unsafe-inline'`); Next reads the nonce from the request CSP and
 * stamps its own bootstrap scripts with it.
 *
 * `connect-src` covers the F2 write (`POST /api/v1/notes`) as well, since it
 * targets that same pinned origin. The pin matters more now than it did when
 * the console was read-only: injected script could otherwise post as the
 * operator, and `serve` trusts localhost.
 *
 * ## Why development needs `'unsafe-eval'`
 *
 * React's DEVELOPMENT build calls `eval()` (it reconstructs call stacks across
 * environments for its debugging features); its production build never does.
 * Without the allowance, `bun run dev` served a page whose scripts all failed
 * silently: React never hydrated, so the board showed frozen skeletons, the
 * connection sat on "connecting" forever, and no control responded — with no
 * failed request to point at, because the client never got as far as fetching.
 *
 * The allowance is therefore gated on `NODE_ENV`, not granted outright. A
 * production build — what `just up` runs — keeps the strict policy it always
 * had.
 */
export default function proxy(request: NextRequest) {
  const nonce = Buffer.from(crypto.randomUUID()).toString("base64");
  const dev = process.env.NODE_ENV === "development";

  const csp = [
    `default-src 'self'`,
    `script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${dev ? " 'unsafe-eval'" : ""}`,
    `style-src 'self' 'unsafe-inline'`,
    `img-src 'self' data:`,
    `font-src 'self'`,
    // The dev server's HMR socket is same-origin but ws:/wss:, which `'self'`
    // does not cover. Portless fronts the console at a named host, so the
    // socket host is not always 127.0.0.1 — allow ws/wss broadly in DEV only;
    // script-src stays nonce-locked.
    `connect-src 'self'${dev ? " ws: wss:" : ""}`,
    `base-uri 'self'`,
    `form-action 'self'`,
    `frame-ancestors 'none'`,
    `object-src 'none'`,
  ].join("; ");

  const requestHeaders = new Headers(request.headers);
  requestHeaders.set("x-nonce", nonce);
  requestHeaders.set("Content-Security-Policy", csp);

  const response = NextResponse.next({ request: { headers: requestHeaders } });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

export const config = {
  matcher: [
    // Everything except Next's own static assets and the favicon.
    {
      source: "/((?!_next/static|_next/image|icon.svg).*)",
      missing: [
        { type: "header", key: "next-router-prefetch" },
        { type: "header", key: "purpose", value: "prefetch" },
      ],
    },
  ],
};
