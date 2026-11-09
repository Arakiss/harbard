import type { NextRequest } from "next/server";
import {
  GET_ROOTS,
  POST_ROOTS,
  UpstreamTokenRefused,
  harbardAuthHeaders,
  harbardUpstreamUrl,
  hostAllowed,
  readAllowed,
  writeAllowed,
} from "@/lib/upstream";

export const dynamic = "force-dynamic";


type Context = { params: Promise<{ path: string[] }> };

function allowed(method: string, path: string[]): boolean {
  const root = path[0] ?? "";
  if (method === "GET" || method === "HEAD") return GET_ROOTS.has(root);
  if (method === "POST") return POST_ROOTS.has(root);
  return false;
}

function sameOriginWrite(request: NextRequest): boolean {
  return writeAllowed({
    host: request.headers.get("host"),
    origin: request.headers.get("origin"),
    protocol: request.nextUrl.protocol,
  });
}

async function forward(request: NextRequest, context: Context): Promise<Response> {
  const { path } = await context.params;
  if (!allowed(request.method, path)) {
    return Response.json({ error: "Harbard API route is not exposed by the console" }, { status: 404 });
  }
  // Reads carry the hub token too: a foreign Host never gets one.
  if (!hostAllowed(request.headers.get("host"))) {
    return Response.json({ error: "This console answers on its own loopback name only" }, { status: 421 });
  }
  if (request.method === "POST" && !sameOriginWrite(request)) {
    return Response.json({ error: "Cross-origin writes are not allowed" }, { status: 403 });
  }
  const origin = { host: request.headers.get("host"), origin: request.headers.get("origin"), protocol: request.nextUrl.protocol };
  if ((request.method === "GET" || request.method === "HEAD") && !readAllowed(path[0] ?? "", origin)) {
    return Response.json({ error: "This read is only served to the console on this Mac" }, { status: 403 });
  }

  const upstream = harbardUpstreamUrl(path.map(encodeURIComponent).join("/"), request.nextUrl.search);
  let headers: Headers;
  try {
    headers = harbardAuthHeaders();
  } catch (error) {
    // Not the local serve, and no HARBARD_HUB_TOKEN for it: nothing is sent.
    if (error instanceof UpstreamTokenRefused) {
      return Response.json({ error: error.message }, { status: 502 });
    }
    throw error;
  }
  const contentType = request.headers.get("content-type");
  if (contentType) headers.set("content-type", contentType);
  headers.set("accept", request.headers.get("accept") ?? "application/json");

  const body = request.method === "POST" ? await request.arrayBuffer() : undefined;
  const response = await fetch(upstream, {
    method: request.method,
    headers,
    body,
    cache: "no-store",
    signal: request.signal,
  });

  const responseHeaders = new Headers();
  // serve's own CSP rides along: an attachment keeps its sandbox.
  for (const name of ["cache-control", "content-type", "retry-after", "content-security-policy"]) {
    const value = response.headers.get(name);
    if (value) responseHeaders.set(name, value);
  }
  responseHeaders.set("x-content-type-options", "nosniff");
  return new Response(request.method === "HEAD" ? null : response.body, {
    status: response.status,
    headers: responseHeaders,
  });
}

export const GET = forward;
export const HEAD = forward;
export const POST = forward;
