/**
 * The README's screenshots and demo, regenerated from the mock serve.
 *
 *   just readme-media                      # build, capture, write assets/screens/
 *   bun run scripts/readme-media.ts --no-build --only=strip,channel
 *
 * `--only=` takes channel, needs-you, deliveries, strip, dark and demo;
 * `--frames=DIR` keeps the demo's raw frames for inspection.
 *
 * Everything comes from `mock/serve.ts` and its `readme` scenario
 * (mock/readme.ts): an invented web shop and its robots. The script never
 * talks to a real `harbard serve`: it starts its own mock and its own
 * `next start` on ports of their own (HARBARD_MOCK_PORT, default 7519, and
 * E2E_PORT, default 3519), refuses to run when either port is taken, and stops
 * both when it ends.
 *
 * The scenario's clock is pinned to a weekday afternoon in October and the
 * mock's season is off, so no seasonal decoration reaches a picture. The demo
 * is a Chromium screencast while the scenario steps forward, assembled into an
 * animated WebP with `img2webp` (libwebp). Stills are WebP too, from 2x PNGs.
 */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, type Browser, type BrowserContext, type Page } from "@playwright/test";

const APP_DIR = dirname(dirname(fileURLToPath(import.meta.url)));
const REPO_DIR = dirname(dirname(APP_DIR));
const OUT_DIR = join(REPO_DIR, "assets", "screens");
const MOCK_PORT = Number(process.env.HARBARD_MOCK_PORT ?? 7519);
const CONSOLE_PORT = Number(process.env.E2E_PORT ?? 3519);
const MOCK = `http://127.0.0.1:${MOCK_PORT}`;
const CONSOLE = `http://127.0.0.1:${CONSOLE_PORT}`;
/** The scenario's day, for the season check: outside 1 Dec – 6 Jan. */
const TODAY = "2026-10-14";

const args = process.argv.slice(2);
const build = !args.includes("--no-build");
const only = args.find((a) => a.startsWith("--only="))?.slice("--only=".length).split(",") ?? null;
/** `--frames=DIR` keeps the demo's raw frames there, to look at them one by one. */
const framesArg = args.find((a) => a.startsWith("--frames="))?.slice("--frames=".length);
const keepFrames = framesArg ? resolve(framesArg) : null;
const wanted = (name: string) => !only || only.includes(name);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function run(cmd: string, argv: string[], env: Record<string, string> = {}): string {
  const out = spawnSync(cmd, argv, { cwd: APP_DIR, env: { ...process.env, ...env }, encoding: "utf8" });
  if (out.status !== 0) throw new Error(`${cmd} ${argv.join(" ")} failed:\n${out.stderr || out.stdout}`);
  return out.stdout.trim();
}

async function portFree(port: number): Promise<boolean> {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(1000) });
    return false;
  } catch {
    return true;
  }
}

async function waitFor(url: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    await sleep(200);
  }
  throw new Error(`${url} did not answer within ${timeoutMs} ms`);
}

/** Its own process group, so stopping it also stops what it spawned. */
function start(cmd: string, argv: string[], env: Record<string, string>): ChildProcess {
  return spawn(cmd, argv, { cwd: APP_DIR, env: { ...process.env, ...env }, stdio: "ignore", detached: true });
}

function stop(proc: ChildProcess | null): void {
  if (!proc?.pid) return;
  try {
    process.kill(-proc.pid, "SIGTERM");
  } catch {
    /* already gone */
  }
}

async function control(path: string): Promise<Record<string, unknown>> {
  const res = await fetch(`${MOCK}/__control/${path}`, { method: "POST" });
  if (!res.ok) throw new Error(`control ${path} -> ${res.status}`);
  return (await res.json()) as Record<string, unknown>;
}

/** The scenario from its first step, in a theme mode, with the season off. */
async function scenario(mode: "light" | "dark", step: number): Promise<number> {
  await control("reset?scenario=readme");
  await control(`theme?mode=${mode}&season=off&decorations=off&today=${TODAY}`);
  const answer = await control(`readme?step=${step}`);
  return Number(answer.now_ms);
}

async function context(browser: Browser, viewport: { width: number; height: number }, scale: number, mode: "light" | "dark"): Promise<BrowserContext> {
  return browser.newContext({
    viewport,
    deviceScaleFactor: scale,
    colorScheme: mode,
    locale: "en-US",
    timezoneId: "UTC",
    reducedMotion: "no-preference",
  });
}

/** Open a page on the scenario's clock. */
async function open(ctx: BrowserContext, now: number, path: string): Promise<Page> {
  const page = await ctx.newPage();
  await page.clock.install({ time: now });
  await page.goto(`${CONSOLE}${path}`);
  await page.waitForLoadState("networkidle").catch(() => undefined);
  // Fonts, robot images and the first activity pull.
  await page.evaluate(() => document.fonts.ready);
  await sleep(2500);
  return page;
}

/** A 2x PNG, written as WebP: sharp text at a fraction of the size. */
async function still(page: Page, name: string, tmp: string): Promise<void> {
  const png = join(tmp, `${name}.png`);
  await page.screenshot({ path: png, animations: "allow" });
  const out = join(OUT_DIR, `${name}.webp`);
  run("cwebp", ["-quiet", "-q", "90", "-sharp_yuv", "-m", "6", png, "-o", out]);
  console.log(`${name}.webp  ${Math.round(statSync(out).size / 1024)} KB`);
}

async function stills(browser: Browser, tmp: string): Promise<void> {
  const wide = { width: 1440, height: 900 };
  if (wanted("channel")) {
    const now = await scenario("light", 4);
    const ctx = await context(browser, wide, 2, "light");
    const page = await open(ctx, now, "/c/storefront");
    await still(page, "channel", tmp);
    await ctx.close();
  }
  if (wanted("needs-you")) {
    const now = await scenario("light", 4);
    const ctx = await context(browser, { width: 1440, height: 720 }, 2, "light");
    const page = await open(ctx, now, "/activity");
    await still(page, "needs-you", tmp);
    await ctx.close();
  }
  if (wanted("deliveries")) {
    const now = await scenario("light", 4);
    const ctx = await context(browser, wide, 2, "light");
    const page = await open(ctx, now, "/deliveries");
    await still(page, "deliveries", tmp);
    await ctx.close();
  }
  if (wanted("strip")) {
    // The strip is made for a 2560×720 display: a 32:9 band.
    const now = await scenario("light", 4);
    const ctx = await context(browser, { width: 1280, height: 360 }, 2, "light");
    const page = await open(ctx, now, "/pulse");
    await still(page, "strip", tmp);
    await ctx.close();
  }
  if (wanted("dark")) {
    const now = await scenario("dark", 4);
    const ctx = await context(browser, wide, 2, "dark");
    const page = await open(ctx, now, "/direct/r05");
    await still(page, "direct-dark", tmp);
    await ctx.close();
  }
}

/** The social card's words, kept apart from the layout so changing them needs no code. */
interface CardCopy {
  headline: string;
  bullets: string[];
}

/**
 * The repository's social preview (GitHub asks for 1280×640): the mark, a
 * headline and three bullets on the left, the channel screenshot bleeding off
 * the right edge. Drawn on a console page so it uses the console's own fonts
 * and colours.
 */
async function card(browser: Browser, tmp: string): Promise<void> {
  const copy = JSON.parse(readFileSync(join(REPO_DIR, "assets", "brand", "social-card.json"), "utf8")) as CardCopy;
  const mark = readFileSync(join(REPO_DIR, "assets", "brand", "mark.svg"), "utf8");
  const shot = join(tmp, "channel.png");
  if (!existsSync(shot)) throw new Error("the card needs the channel still: run with --only=channel,card or without --only");
  const screenshot = `data:image/png;base64,${readFileSync(shot).toString("base64")}`;
  const now = await scenario("light", 4);
  const ctx = await context(browser, { width: 1280, height: 640 }, 1, "light");
  const page = await open(ctx, now, "/c/storefront");
  await page.evaluate(
    ({ copy, mark, screenshot }) => {
      const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);
      document.body.innerHTML = `
        <main class="card">
          <section class="words">
            <div class="brand"><span class="mark">${mark}</span><span class="name">harbard</span></div>
            <h1>${esc(copy.headline)}</h1>
            <ul>${copy.bullets.map((b) => `<li>${esc(b)}</li>`).join("")}</ul>
          </section>
          <figure class="shot"><img src="${screenshot}" alt=""></figure>
        </main>`;
      const style = document.createElement("style");
      style.textContent = `
        html, body { margin: 0; width: 1280px; height: 640px; overflow: hidden; }
        .card { position: relative; width: 1280px; height: 640px; overflow: hidden;
          background: radial-gradient(120% 140% at 0% 0%, #fffaf3 0%, #f6ecdf 55%, #efe2d1 100%);
          font-family: var(--font-sans); color: #2a241d; }
        .words { position: absolute; left: 72px; top: 76px; width: 520px; }
        .brand { display: flex; align-items: center; gap: 14px; }
        .mark svg { width: 64px; height: 64px; color: #2a241d; display: block; }
        .name { font-size: 46px; font-weight: 700; letter-spacing: -1.5px; }
        h1 { margin: 44px 0 0; font-size: 50px; line-height: 1.08; font-weight: 700; letter-spacing: -1.6px; text-wrap: balance; }
        ul { margin: 36px 0 0; padding: 0; list-style: none; }
        li { position: relative; padding-left: 26px; margin: 0 0 14px; font-size: 25px; line-height: 1.3; color: #4a4036; }
        li::before { content: ""; position: absolute; left: 2px; top: 11px; width: 10px; height: 10px; border-radius: 50%; background: var(--hb-accent, #c4503a); }
        .shot { position: absolute; left: 640px; top: 78px; margin: 0; width: 900px; height: 562px;
          border-radius: 18px 0 0 0; overflow: hidden; box-shadow: 0 24px 60px rgba(60, 40, 20, .22), 0 0 0 1px rgba(60, 40, 20, .08); }
        .shot img { width: 900px; height: auto; display: block; }`;
      document.head.appendChild(style);
    },
    { copy, mark, screenshot },
  );
  await page.evaluate(() => document.fonts.ready);
  await sleep(300);
  const out = join(REPO_DIR, "assets", "social-preview.png");
  await page.screenshot({ path: out });
  console.log(`social-preview.png  ${Math.round(statSync(out).size / 1024)} KB`);
  await ctx.close();
}

interface Frame {
  file: string;
  ts: number;
}

/**
 * The demo: a screencast of the channel while the scenario steps forward.
 * Frames arrive when the page paints; each is held until the next one.
 */
async function demo(browser: Browser, tmp: string): Promise<void> {
  const now = await scenario("light", 0);
  const ctx = await context(browser, { width: 1280, height: 800 }, 1, "light");
  const page = await open(ctx, now, "/c/storefront");
  const cdp = await ctx.newCDPSession(page);
  const frames: Frame[] = [];
  const dir = join(tmp, "frames");
  mkdirSync(dir, { recursive: true });
  cdp.on("Page.screencastFrame", (f: { data: string; sessionId: number; metadata: { timestamp?: number } }) => {
    // Ack first: the next frame is not sent until this one is acknowledged.
    void cdp.send("Page.screencastFrameAck", { sessionId: f.sessionId }).catch(() => undefined);
    const file = join(dir, `${String(frames.length).padStart(5, "0")}.jpg`);
    writeFileSync(file, Buffer.from(f.data, "base64"));
    frames.push({ file, ts: (f.metadata.timestamp ?? Date.now() / 1000) * 1000 });
  });
  await cdp.send("Page.startScreencast", { format: "jpeg", quality: 95, everyNthFrame: 1 });
  const began = Date.now();
  const at = async (ms: number) => {
    const wait = began + ms - Date.now();
    if (wait > 0) await sleep(wait);
  };
  // The channel as it is; a finding arrives; the robots move on to new work;
  // one asks a question; a delivery lands; the operator answers the question
  // and the blocked robot goes back to work.
  await at(2000);
  await control("readme?step=1");
  await at(4500);
  await control("readme?step=2");
  await at(7000);
  await control("readme?step=3");
  await at(9500);
  await control("readme?step=4");
  await at(11000);
  const composer = page.getByPlaceholder("Message #storefront");
  await composer.click();
  await composer.pressSequentially("migrations: go ahead now, traffic stays low until 16:00.", { delay: 28 });
  await at(13500);
  await composer.press("Enter");
  await at(14500);
  await control("readme?step=5");
  await at(17000);
  await cdp.send("Page.stopScreencast");
  await ctx.close();
  if (frames.length < 2) throw new Error(`the screencast caught ${frames.length} frames`);
  if (keepFrames) {
    mkdirSync(keepFrames, { recursive: true });
    for (const f of frames) copyFileSync(f.file, join(keepFrames, basename(f.file)));
  }

  // Hold each frame until the next (the last for a beat before the loop);
  // fold frames shorter than 40 ms into the one after them.
  const argv = ["-loop", "0", "-lossy", "-q", "80", "-m", "6"];
  let kept = 0;
  let held = 0;
  for (let i = 0; i < frames.length; i += 1) {
    const next = frames[i + 1]?.ts;
    const hold = next === undefined ? 1500 : Math.round(next - frames[i]!.ts) + held;
    if (hold < 40 && next !== undefined) {
      held = hold;
      continue;
    }
    held = 0;
    argv.push("-d", String(hold), frames[i]!.file);
    kept += 1;
  }
  const out = join(OUT_DIR, "demo.webp");
  argv.push("-o", out);
  run("img2webp", argv);
  console.log(`demo.webp  ${Math.round(statSync(out).size / 1024)} KB, ${kept} frames`);
}

async function main(): Promise<void> {
  for (const port of [MOCK_PORT, CONSOLE_PORT]) {
    if (!(await portFree(port))) throw new Error(`port ${port} is taken; set HARBARD_MOCK_PORT / E2E_PORT to free ones`);
  }
  const commit = run("git", ["rev-parse", "--short", "HEAD"]);
  if (build) {
    console.log("building the console…");
    run("bun", ["run", "build"], { NEXT_PUBLIC_HARBARD_COMMIT: commit });
  }
  mkdirSync(OUT_DIR, { recursive: true });
  const tmp = mkdtempSync(join(tmpdir(), "harbard-readme-media-"));
  let mock: ChildProcess | null = null;
  let next: ChildProcess | null = null;
  let browser: Browser | null = null;
  try {
    mock = start("bun", ["run", "mock/serve.ts"], {
      PORT: String(MOCK_PORT),
      HARBARD_FIXTURE: "readme",
      HARBARD_MOCK_UPDATE: "current",
      HARBARD_MOCK_RUNNING_COMMIT: commit,
    });
    await waitFor(`${MOCK}/__control/ready`, 10_000);
    next = start("bun", ["x", "next", "start", "--hostname", "127.0.0.1", "--port", String(CONSOLE_PORT)], {
      HARBARD_API: MOCK,
      // The mock is not the local serve: the proxy sends it only this token.
      HARBARD_HUB_TOKEN: "e2e-mock-fixture-not-a-secret",
      HARBARD_MOCK_PORT: String(MOCK_PORT),
    });
    await waitFor(CONSOLE, 60_000);
    browser = await chromium.launch();
    await stills(browser, tmp);
    if (wanted("card")) await card(browser, tmp);
    if (wanted("demo")) await demo(browser, tmp);
  } finally {
    await browser?.close();
    stop(next);
    stop(mock);
    rmSync(tmp, { recursive: true, force: true });
  }
}

await main();
