import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { harbardAuthHeaders, localServe, UpstreamTokenRefused } from "./upstream";

// A throwaway console must not send the on-disk token upstream: that token
// goes only to the local serve; any other upstream needs HARBARD_HUB_TOKEN.

const FAKE_DISK_TOKEN = "disk-token-fixture-not-a-secret";
const saved = {
  api: process.env.HARBARD_API,
  token: process.env.HARBARD_HUB_TOKEN,
  home: process.env.HARBARD_HOME,
  servePort: process.env.HARBARD_API_PORT,
  port: process.env.PORT,
};
const homes: string[] = [];

function restore(key: string, value: string | undefined) {
  if (value === undefined) delete process.env[key];
  else process.env[key] = value;
}

afterEach(() => {
  restore("HARBARD_API", saved.api);
  restore("HARBARD_HUB_TOKEN", saved.token);
  restore("HARBARD_HOME", saved.home);
  restore("HARBARD_API_PORT", saved.servePort);
  restore("PORT", saved.port);
  for (const home of homes.splice(0)) rmSync(home, { recursive: true, force: true });
});

/** A throwaway harbard home with a fake token on disk, and no env token. */
function fakeHome() {
  const home = mkdtempSync(join(tmpdir(), "harbard-console-upstream-"));
  homes.push(home);
  writeFileSync(join(home, "hub.token"), `${FAKE_DISK_TOKEN}\n`, { mode: 0o600 });
  process.env.HARBARD_HOME = home;
  delete process.env.HARBARD_HUB_TOKEN;
  delete process.env.HARBARD_API_PORT;
}

describe("the token on disk goes only to the local serve", () => {
  test("a mock upstream on another port gets no token and a refusal", () => {
    fakeHome();
    process.env.HARBARD_API = "http://127.0.0.1:7461";
    expect(() => harbardAuthHeaders()).toThrow(UpstreamTokenRefused);
    try {
      harbardAuthHeaders();
    } catch (error) {
      const message = (error as Error).message;
      expect(message).toContain("HARBARD_HUB_TOKEN");
      expect(message).toContain("127.0.0.1:7461");
      expect(message).not.toContain(FAKE_DISK_TOKEN);
    }
  });

  test("so does any host that is not loopback, https, or another port", () => {
    fakeHome();
    for (const api of [
      "http://192.168.1.10:7419",
      "http://harbard.example:7419",
      "https://127.0.0.1:7419",
      "http://127.0.0.1",
    ]) {
      process.env.HARBARD_API = api;
      expect(() => harbardAuthHeaders()).toThrow(UpstreamTokenRefused);
    }
  });

  test("the local serve, by default or by name, still gets it", () => {
    fakeHome();
    for (const api of [undefined, "http://127.0.0.1:7419", "http://localhost:7419/", "http://[::1]:7419"]) {
      restore("HARBARD_API", api);
      expect(harbardAuthHeaders().get("authorization")).toBe(`Bearer ${FAKE_DISK_TOKEN}`);
    }
  });

  test("serve on the port the LaunchAgents were given counts as local", () => {
    fakeHome();
    process.env.HARBARD_API_PORT = "7500";
    process.env.HARBARD_API = "http://127.0.0.1:7500";
    expect(harbardAuthHeaders().get("authorization")).toBe(`Bearer ${FAKE_DISK_TOKEN}`);
    process.env.HARBARD_API = "http://127.0.0.1:7419";
    expect(() => harbardAuthHeaders()).toThrow(UpstreamTokenRefused);
  });

  test("an explicit HARBARD_HUB_TOKEN goes where HARBARD_API says", () => {
    fakeHome();
    process.env.HARBARD_API = "http://127.0.0.1:7461";
    process.env.HARBARD_HUB_TOKEN = "explicit-fixture";
    expect(harbardAuthHeaders().get("authorization")).toBe("Bearer explicit-fixture");
  });

  test("localServe is plain http on a loopback name and serve's port", () => {
    delete process.env.HARBARD_API_PORT;
    expect(localServe(new URL("http://127.0.0.1:7419/api/v1/"))).toBe(true);
    expect(localServe(new URL("http://127.0.0.1:7461/api/v1/"))).toBe(false);
    expect(localServe(new URL("http://127.0.0.2:7419/api/v1/"))).toBe(false);
  });
});

describe("the proxy refuses instead of forwarding", () => {
  test("a mock upstream on :7461 receives nothing at all", async () => {
    fakeHome();
    process.env.HARBARD_API = "http://127.0.0.1:7461";
    delete process.env.PORT;
    const sent: { url: string; authorization: string | null }[] = [];
    const realFetch = globalThis.fetch;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      sent.push({ url: String(input), authorization: new Headers(init?.headers).get("authorization") });
      return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
    }) as typeof fetch;
    try {
      const { GET } = await import("@/app/api/harbard/[...path]/route");
      const request = {
        method: "GET",
        headers: new Headers({ host: "127.0.0.1:3419" }),
        nextUrl: new URL("http://127.0.0.1:3419/api/harbard/workspace"),
        signal: new AbortController().signal,
      };
      const response = await GET(request as never, { params: Promise.resolve({ path: ["workspace"] }) });
      expect(response.status).toBe(502);
      const body = (await response.json()) as { error: string };
      expect(body.error).toContain("HARBARD_HUB_TOKEN");
      expect(sent).toEqual([]);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
