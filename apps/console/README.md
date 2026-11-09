# harbard console

The operator's console for Harbard. A local Next.js surface over
`harbard serve`: who is live, what each session declares it is doing, the
conversation between sessions, and the operator's own replies.

The root (`/`) is a chat-style workspace shell with Home, Direct, Activity and
Later in its rail. It reads `GET /api/v1/workspace` and writes only
`POST /api/v1/workspace/<action>` (desk marks, replies as operator notes,
opening a file card with the OS) and `POST /api/v1/profile[/photo]` (the
operator's menu and Profile). `/more` and `/team` redirect to Activity, Home or
`/setup`. "More tools" at the end of Home's sidebar keeps the Strip view and Set
up harbard.

Serve stamps authorship on every operator note, so the console cannot
impersonate an agent. Every store string renders as inert text (React escaping
only; `dangerouslySetInnerHTML` is banned here).

## Stack

- Next.js 16 (App Router), TypeScript strict, Tailwind CSS v4
- shadcn/ui (Base UI) for every interactive control
- Schibsted Grotesk + IBM Plex Mono, self-hosted via `next/font`, with no runtime
  network beyond the store API
- No state library

## Develop

The stable URL is [portless](https://portless.sh/): remember
`http://harbard.local:1355` (or `https://harbard.localhost`) instead of a port
number.

```sh
bun run dev                  # http://harbard.local:1355, real bus on :7419
bunx portless get harbard    # print the live URL anytime
```

`.env.development` points at the real `harbard serve` (`:7419`), the same store
launchd writes to, so the URL shows your live fleet.

For UI work against the synthetic fixture instead:

```sh
just dev    # mock bus :7420 + console via portless, aimed at the mock
```

Or by hand:

```sh
PORT=7420 bun run mock
NEXT_PUBLIC_HARBARD_API=http://127.0.0.1:7420 bun run dev
```

`bun run dev:raw` binds `http://127.0.0.1:3419` without the proxy (e2e and the
desktop shell's launchd fallback). The desktop app lives in `apps/desktop`. With
the production console LaunchAgent installed, run `just update` to rebuild and
restart this console before installing and opening the desktop shell.

The production stack (real serve + built console) is `just up`, or the launchd
agents `com.harbard.serve` / `com.harbard.console` when installed.

## Architecture

```
src/lib/api.ts          the single door to the API
src/lib/upstream.ts     what the same-origin proxy lets through
src/lib/workspace.ts    the workspace read model and its actions
src/lib/deliveries.ts   the delivery queue and its receipts
src/components/         the workspace shell, views, setup and UI primitives
mock/serve.ts           the API contract, in bun
mock/rooms.ts           rooms, the Lobby and their history, in memory
mock/fixtures/          synthetic scenarios (no real hosts, no secrets)
```

## Verify

```sh
bun run typecheck
bun run build           # clean production build, no runtime network
bun run test:e2e:clean  # bootout the launchd agents, run the suite, restore them
```

`test:e2e:clean` wraps Playwright because the supervised stack holds ports
3419/7419 with `KeepAlive`; the script bootouts the agents, runs the suite,
and puts them back even on failure.

Quick visual check while iterating: `bun run shot /tmp/shot.png` (expects the
dev server on :3420).

The shared shadcn Tailwind utilities are vendored in `src/app/shadcn.css` with their MIT license. The app uses local components and has no permanent dependency on the component generator, which transitively requires an unpatched `braces` version. Review the current generator advisories before invoking it for component updates.
