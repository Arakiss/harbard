import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  GET_ROOTS,
  POST_ROOTS,
  harbardAuthHeaders,
  harbardUpstreamUrl,
  upstreamRoot,
  readAllowed,
  hostAllowed,
  namedHostAllowed,
  writeAllowed,
} from "./upstream";

const originalApi = process.env.HARBARD_API;
const originalToken = process.env.HARBARD_HUB_TOKEN;
const originalHome = process.env.HARBARD_HOME;
const temporaryHomes: string[] = [];

afterEach(() => {
  if (originalApi === undefined) delete process.env.HARBARD_API;
  else process.env.HARBARD_API = originalApi;
  if (originalToken === undefined) delete process.env.HARBARD_HUB_TOKEN;
  else process.env.HARBARD_HUB_TOKEN = originalToken;
  if (originalHome === undefined) delete process.env.HARBARD_HOME;
  else process.env.HARBARD_HOME = originalHome;
  for (const path of temporaryHomes.splice(0)) rmSync(path, { recursive: true, force: true });
});

describe("Harbard server-side upstream", () => {
  test("uses the loopback API by default", () => {
    delete process.env.HARBARD_API;
    expect(harbardUpstreamUrl("sessions").toString()).toBe(
      "http://127.0.0.1:7419/api/v1/sessions",
    );
  });

  test("never forwards a query-string token", () => {
    const url = harbardUpstreamUrl("stream", "?token=secret&after=01ABC");
    expect(url.searchParams.has("token")).toBe(false);
    expect(url.searchParams.get("after")).toBe("01ABC");
  });

  test("rejects credentials embedded in HARBARD_API", () => {
    process.env.HARBARD_API = "http://user:password@127.0.0.1:7419";
    expect(() => harbardUpstreamUrl("sessions")).toThrow("must not contain credentials");
  });

  test("adds the hub token only to server-side headers", () => {
    process.env.HARBARD_HUB_TOKEN = "test-token";
    expect(harbardAuthHeaders().get("authorization")).toBe("Bearer test-token");
  });

  test("reads the private local token when no environment override exists", () => {
    delete process.env.HARBARD_HUB_TOKEN;
    delete process.env.HARBARD_API;
    const home = mkdtempSync(join(tmpdir(), "harbard-console-token-"));
    temporaryHomes.push(home);
    mkdirSync(home, { recursive: true });
    writeFileSync(join(home, "hub.token"), "file-token\n", { mode: 0o600 });
    process.env.HARBARD_HOME = home;
    expect(harbardAuthHeaders().get("authorization")).toBe("Bearer file-token");
  });
});

describe("the proxy allowlist covers what the client actually asks for", () => {
  // Every path `api.ts` reaches for, read from the source rather than
  // hand-listed: a hand-listed copy drifts exactly the way the allowlist did.
  const source = readFileSync(join(import.meta.dir, "api.ts"), "utf8");

  const pathsFor = (fn: "get" | "post"): string[] => {
    const calls = [...source.matchAll(new RegExp(`\\b${fn}(?:<[^>]*>)?\\(\\s*\`?"?([^"\`,)]+)`, "g"))];
    return calls
      .map((m) => m[1] ?? "")
      .filter((p) => p.startsWith("/"))
      .map(upstreamRoot)
      .map((r) => r.split("$")[0] ?? r)
      .filter(Boolean);
  };

  /**
   * The writes, read the same way — from the source.
   *
   * Reads go through `get()`, so `pathsFor("get")` finds them. Writes each call
   * `fetch` directly, so they need their own sweep: without one, `POST
   * /operator` sat outside the allowlist and the operator profile write was
   * proxied to nothing, which is exactly the silence the GET test exists to
   * break.
   */
  const postPaths = (): string[] =>
    [...source.matchAll(/fetch\(`\$\{API_V1\}(\/[^`]*)`,\s*\{\s*\n\s*method: "POST"/g)]
      .map((m) => upstreamRoot(m[1] ?? ""))
      .filter(Boolean);

  test("every GET the console makes has an allowed root", () => {
    const missing = [...new Set(pathsFor("get"))].filter((r) => !GET_ROOTS.has(r));
    expect(missing).toEqual([]);
  });

  test("history is reachable — its absence made the whole tab 404", () => {
    expect(GET_ROOTS.has("history")).toBe(true);
    expect(upstreamRoot("/history/sessions")).toBe("history");
    expect(upstreamRoot("/history/checkouts")).toBe("history");
    expect(upstreamRoot("/history?limit=40&kinds=note%2Cack")).toBe("history");
  });

  test("note delivery trace is a GET on the notes root", () => {
    expect(GET_ROOTS.has("notes")).toBe(true);
    expect(upstreamRoot("/notes/01ABC/trace")).toBe("notes");
  });

  test("every POST the console makes has an allowed root", () => {
    const found = [...new Set(postPaths())];
    expect(found.length).toBeGreaterThan(0);
    expect(found.filter((r) => !POST_ROOTS.has(r))).toEqual([]);
  });

  test("the operator's read cursors ride the operator root, never cursors/", () => {
    expect(upstreamRoot("/operator/reads")).toBe("operator");
    expect(POST_ROOTS.has("cursors")).toBe(false);
    expect(GET_ROOTS.has("cursors")).toBe(false);
  });

  test("writes stay narrower than reads", () => {
    for (const root of POST_ROOTS) expect(GET_ROOTS.has(root) || root === "notes").toBe(true);
  });
});

describe("writes are accepted only from the console's own loopback host", () => {
  // Low 4 of the review: a DNS-rebinding page sends a Host and an Origin that
  // agree on its own name; that agreement alone must never be enough.
  test("a rebinding host is refused even when Origin matches it", () => {
    expect(writeAllowed({ host: "evil.example:3519", origin: "http://evil.example:3519", protocol: "http:" })).toBe(false);
    expect(writeAllowed({ host: "evil.example:3519", origin: null, protocol: "http:" })).toBe(false);
    expect(writeAllowed({ host: "127.0.0.1.evil.example:3419", origin: null, protocol: "http:" })).toBe(false);
    expect(writeAllowed({ host: null, origin: null, protocol: "http:" })).toBe(false);
  });

  test("127.0.0.1:<port> and localhost:<port> are accepted", () => {
    expect(writeAllowed({ host: "127.0.0.1:3419", origin: "http://127.0.0.1:3419", protocol: "http:" })).toBe(true);
    expect(writeAllowed({ host: "localhost:3419", origin: null, protocol: "http:" })).toBe(true);
    expect(writeAllowed({ host: "127.0.0.1", origin: null, protocol: "http:" })).toBe(false);
  });

  test("a cross-origin write to the loopback host is still refused", () => {
    expect(writeAllowed({ host: "127.0.0.1:3419", origin: "http://evil.example", protocol: "http:" })).toBe(false);
  });
});

describe("private reads", () => {
  test("the live view's prose is served only to a loopback Host", () => {
    const rebinding = { host: "evil.example:3419", origin: null, protocol: "http:" };
    expect(readAllowed("live", rebinding)).toBe(false);
    expect(readAllowed("live", { host: "127.0.0.1:3419", origin: "http://evil.example", protocol: "http:" })).toBe(false);
    expect(readAllowed("live", { host: "127.0.0.1:3419", origin: null, protocol: "http:" })).toBe(true);
    expect(readAllowed("live", { host: "localhost:3419", origin: "http://localhost:3419", protocol: "http:" })).toBe(true);
    // History carries a month of agents' prose: the same rule as live.
    expect(readAllowed("history", rebinding)).toBe(false);
    expect(readAllowed("history", { host: "127.0.0.1:3419", origin: "http://evil.example", protocol: "http:" })).toBe(false);
    expect(readAllowed("history", { host: "127.0.0.1:3419", origin: null, protocol: "http:" })).toBe(true);
    // Every read now needs the console's own Host: one helper.
    expect(readAllowed("workspace", rebinding)).toBe(false);
    expect(readAllowed("workspace", { host: "127.0.0.1:3419", origin: "http://elsewhere", protocol: "http:" })).toBe(true);
  });
});

describe("every proxied request needs the console's own Host", () => {
  test("loopback names with a port pass; a rebinding name never does, reads included", () => {
    for (const host of ["127.0.0.1:3419", "localhost:3419", "[::1]:3419"]) expect(hostAllowed(host)).toBe(true);
    for (const host of ["evil.example:3419", "127.0.0.1.evil.example:3419", "127.0.0.1", null, ""]) expect(hostAllowed(host)).toBe(false);
  });

  test("the proxy route checks Host before it forwards any method, and keeps serve's CSP", async () => {
    const source = await Bun.file(new URL("../app/api/harbard/[...path]/route.ts", import.meta.url)).text();
    const check = source.indexOf("hostAllowed(request.headers.get(\"host\"))");
    expect(check).toBeGreaterThan(0);
    expect(check).toBeLessThan(source.indexOf("await fetch(upstream"));
    expect(source).toContain('"content-security-policy"');
  });
});

describe("the named dev host is an explicit opt-in (HARBARD_ALLOWED_HOSTS)", () => {
  const saved = process.env.HARBARD_ALLOWED_HOSTS;
  afterEach(() => {
    if (saved === undefined) delete process.env.HARBARD_ALLOWED_HOSTS;
    else process.env.HARBARD_ALLOWED_HOSTS = saved;
  });

  test("off by default: harbard.localhost gets nothing", () => {
    delete process.env.HARBARD_ALLOWED_HOSTS;
    expect(hostAllowed("harbard.localhost:1355")).toBe(false);
    expect(readAllowed("workspace", { host: "harbard.localhost", origin: null, protocol: "http:" })).toBe(false);
  });

  test("opted in: exactly the named hosts, nothing that merely contains them", () => {
    process.env.HARBARD_ALLOWED_HOSTS = "harbard.localhost, foo.local";
    expect(hostAllowed("harbard.localhost:1355")).toBe(true);
    expect(hostAllowed("foo.local")).toBe(true);
    expect(namedHostAllowed("harbard.localhost.evil.example")).toBe(false);
    expect(hostAllowed("evil.example:3419")).toBe(false);
    expect(writeAllowed({ host: "harbard.localhost", origin: "http://harbard.localhost", protocol: "http:" })).toBe(true);
  });
});
