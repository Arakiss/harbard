"use client";

import * as React from "react";

import { CheckCircle2, Wrench } from "lucide-react";
import { BrandMark } from "@/components/brand/brand-mark";
import { createT } from "@/i18n";
import {
  fetchSetup,
  postSetupComplete,
  postSetupInstall,
} from "@/lib/api";
import { SETUP_TONE, harnessLabel } from "@/lib/harness";
import { blurbFor, tools } from "./setup-copy";
import type { SetupHarness, SetupInstallResult } from "@/lib/types";

const t = createT("setup");

type Step = "welcome" | "scan" | "choose" | "wire" | "done";

/**
 * First-run Setup — wire detected harnesses so agents can hear the bus.
 *
 * Agents do not peer-DM. Hooks inject a brief; missing wiring looks like a
 * broken product. This wizard detects Claude / Codex / Cursor, explains why
 * each matters, installs with confirmation, then lets the operator in.
 */
function SetupScreen({
  initial,
  force = false,
  onFinished,
}: {
  initial: { harnesses: SetupHarness[] };
  force?: boolean;
  onFinished: () => void;
}) {
  const [step, setStep] = React.useState<Step>("welcome");
  const [harnesses, setHarnesses] = React.useState(initial.harnesses);
  const [selected, setSelected] = React.useState<Set<string>>(() => {
    const next = new Set<string>();
    for (const h of initial.harnesses) {
      if (h.recommended || (force && !h.ok)) next.add(h.target);
    }
    return next;
  });
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [results, setResults] = React.useState<SetupInstallResult[] | null>(null);
  const [scanTick, setScanTick] = React.useState(0);

  // Real re-detect via GET /setup — same path as `harbard setup --dry-run`.
  React.useEffect(() => {
    if (step !== "scan") return;
    let cancelled = false;
    const ac = new AbortController();
    setScanTick(0);
    setError(null);
    void (async () => {
      try {
        // Stagger reveal while the request is in flight; settle on real data.
        window.setTimeout(() => {
          if (!cancelled) setScanTick(1);
        }, 120);
        window.setTimeout(() => {
          if (!cancelled) setScanTick(2);
        }, 240);
        const s = await fetchSetup(ac.signal);
        if (cancelled) return;
        setScanTick(3);
        setHarnesses(s.harnesses);
        const next = new Set<string>();
        for (const h of s.harnesses) {
          if (h.recommended || (force && !h.ok)) next.add(h.target);
        }
        setSelected(next);
        setStep("choose");
      } catch (e) {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : t("errorScan"));
        setStep("welcome");
      }
    })();
    return () => {
      cancelled = true;
      ac.abort();
    };
  }, [step, force]);

  const toggle = (target: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(target)) next.delete(target);
      else next.add(target);
      return next;
    });
  };

  const finish = async (opts: { skipped: string[]; wired: string[] }) => {
    setBusy(true);
    setError(null);
    try {
      await postSetupComplete(opts);
      setStep("done");
      window.setTimeout(onFinished, 900);
    } catch (e) {
      setError(e instanceof Error ? e.message : t("errorSave"));
      setBusy(false);
    }
  };

  const skipAll = () => {
    void finish({
      skipped: harnesses.map((h) => h.target),
      wired: [],
    });
  };

  const wireSelected = async () => {
    const targets = [...selected];
    if (targets.length === 0) {
      await finish({
        skipped: harnesses.map((h) => h.target),
        wired: [],
      });
      return;
    }
    setBusy(true);
    setError(null);
    setStep("wire");
    try {
      const res = await postSetupInstall(targets);
      setResults(res.results);
      setHarnesses(res.setup.harnesses);
      const wired = res.results.filter((r) => r.ok).map((r) => r.target);
      const failed = res.results.filter((r) => !r.ok);
      // Partial wire must not celebrate — deaf agents look like a broken product.
      if (failed.length > 0) {
        setBusy(false);
        setError(
          failed.length === 1
            ? t("errorWireOne", { tool: harnessLabel(failed[0]!.target) })
            : t("errorWireMany", { count: failed.length }),
        );
        return;
      }
      const skipped = harnesses
        .map((h) => h.target)
        .filter((t) => !wired.includes(t));
      await postSetupComplete({ skipped, wired });
      setBusy(false);
      setStep("done");
      window.setTimeout(onFinished, 1100);
    } catch (e) {
      setError(e instanceof Error ? e.message : t("errorWire"));
      setBusy(false);
      setStep("choose");
    }
  };

  const presentCount = harnesses.filter((h) => h.present).length;
  const wiredCount = harnesses.filter((h) => h.ok).length;

  return (
    <main className="hb-main hb-setup" aria-labelledby="setup-title">
      <header className="hb-head">
        <div className="hb-head-title">
          <Wrench className="size-5" strokeWidth={2.4} aria-hidden />
          {t("title")}
        </div>
        {step !== "done" && step !== "wire" ? (
          <div className="hb-head-right">
            <button type="button" className="hb-btn" onClick={skipAll} disabled={busy}>
              {t("notNow")}
            </button>
          </div>
        ) : null}
      </header>

      <div className="hb-setup-body">
        {step === "welcome" && (
          <section className="hb-setup-card hb-setup-center">
            <span className="hb-setup-mark">
              <BrandMark className="hb-setup-logo" />
            </span>
            <h1 id="setup-title" className="hb-setup-brand">
              harbard
            </h1>
            <p className="hb-setup-lead">{t("welcomeLead")}</p>
            <p className="hb-setup-note">{t("welcomeNote")}</p>
            {error ? (
              <p className="hb-error" role="alert">
                {error}
              </p>
            ) : null}
            <button type="button" className="hb-btn hb-btn-primary hb-setup-go" onClick={() => setStep("scan")}>
              {t("searchThisMac")}
            </button>
          </section>
        )}

        {step === "scan" && (
          <section className="hb-setup-card hb-setup-center" aria-live="polite">
            <h2 id="setup-title" className="hb-setup-title">
              {t("searching")}
            </h2>
            <p className="hb-setup-note">{t("scanList")}</p>
            <ul className="hb-setup-list">
              {["claude", "codex", "cursor"].map((target, i) => {
                const h = harnesses.find((x) => x.target === target);
                const revealed = scanTick > i;
                return (
                  <li key={target} className="hb-setup-row" data-revealed={revealed || undefined}>
                    <span className="hb-setup-row-name">{harnessLabel(h?.harness ?? target)}</span>
                    <span className="hb-chip" data-tone={revealed && h?.present ? SETUP_TONE.found : SETUP_TONE.missing}>
                      {revealed ? (h?.present ? t("found") : t("missing")) : "…"}
                    </span>
                  </li>
                );
              })}
            </ul>
          </section>
        )}

        {step === "choose" && (
          <section className="hb-setup-card">
            <h2 id="setup-title" className="hb-setup-title">
              {presentCount > 0 ? t("chooseFound", { tools: tools(presentCount) }) : t("chooseNone")}
            </h2>
            <p className="hb-setup-note">
              {presentCount > 0
                ? t("chooseHint")
                : t("chooseNoneHint")}
            </p>
            <ul className="hb-setup-list">
              {harnesses.map((h) => (
                <HarnessCard key={h.target} harness={h} selected={selected.has(h.target)} onToggle={() => toggle(h.target)} />
              ))}
            </ul>
            {error ? (
              <p className="hb-error" role="alert">
                {error}
              </p>
            ) : null}
            <div className="hb-setup-actions">
              <button
                type="button"
                className={selected.size === 0 ? "hb-btn" : "hb-btn hb-btn-primary"}
                disabled={busy}
                onClick={() => void wireSelected()}
              >
                {selected.size === 0 ? t("notNow") : t("connectCount", { count: selected.size })}
              </button>
              <span className="hb-setup-count">
                {t("alreadyConnected", { wired: wiredCount, total: harnesses.length })}
              </span>
            </div>
          </section>
        )}

        {step === "wire" && (
          <section className="hb-setup-card hb-setup-center" aria-live="polite">
            <h2 id="setup-title" className="hb-setup-title">
              {error ? t("wireIncompleteTitle") : t("wireTitle")}
            </h2>
            <p className="hb-setup-note">
              {error
                ? t("wireIncompleteNote")
                : t("wireNote")}
            </p>
            {error ? (
              <p className="hb-error" role="alert">
                {error}
              </p>
            ) : null}
            {results ? (
              <ul className="hb-setup-list">
                {results.map((r) => (
                  <li key={r.target} className="hb-setup-row" data-revealed>
                    <span className="hb-setup-row-name">{harnessLabel(r.target)}</span>
                    <span className="hb-chip" data-tone={r.ok ? SETUP_TONE.wired : SETUP_TONE.failed}>
                      {r.ok ? t("connected") : t("failed")}
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
            {error ? (
              <div className="hb-setup-actions">
                <button
                  type="button"
                  className="hb-btn hb-btn-primary"
                  onClick={() => {
                    setError(null);
                    setResults(null);
                    setStep("choose");
                  }}
                >
                  {t("chooseAgain")}
                </button>
                <button type="button" className="hb-btn" disabled={busy} onClick={() => void wireSelected()}>
                  {t("retry")}
                </button>
              </div>
            ) : null}
          </section>
        )}

        {step === "done" && (
          <section className="hb-setup-card hb-setup-center">
            <span className="hb-setup-done" aria-hidden>
              <CheckCircle2 className="size-10" strokeWidth={2} />
            </span>
            <h2 id="setup-title" className="hb-setup-title">
              {t("doneTitle")}
            </h2>
            <p className="hb-setup-note">{t("doneNote")}</p>
          </section>
        )}
      </div>
    </main>
  );
}

function HarnessCard({
  harness,
  selected,
  onToggle,
}: {
  harness: SetupHarness;
  selected: boolean;
  onToggle: () => void;
}) {
  const label = harnessLabel(harness.harness);
  const id = `hb-setup-${harness.target}`;
  return (
    <li className="hb-setup-tool" data-selected={selected || undefined}>
      <input id={id} type="checkbox" className="hb-setup-check" checked={selected} onChange={onToggle} />
      <label htmlFor={id} className="hb-setup-tool-text">
        <span className="hb-setup-tool-head">
          <span className="hb-setup-tool-name">{label}</span>
          <span className="hb-chip" data-tone={harness.ok ? SETUP_TONE.wired : harness.present ? SETUP_TONE.detected : SETUP_TONE.missing}>
            {harness.ok ? t("connected") : harness.present ? t("detected") : t("missing")}
          </span>
        </span>
        <span className="hb-setup-tool-blurb">{blurbFor(harness.target, harness.blurb)}</span>
        <span className="hb-mono hb-setup-tool-path">{harness.path}</span>
      </label>
    </li>
  );
}

/**
 * Gate: fetch setup status, show the wizard when needed (or when `?setup=1`).
 */
export function SetupGate({ children, always = false }: { children: React.ReactNode; always?: boolean }) {
  const [state, setState] = React.useState<
    | { kind: "loading" }
    | { kind: "ready"; show: false }
    | { kind: "ready"; show: true; harnesses: SetupHarness[]; force: boolean }
  >({ kind: "loading" });

  React.useEffect(() => {
    const force =
      always ||
      (typeof window !== "undefined" &&
        new URLSearchParams(window.location.search).get("setup") === "1");
    const ac = new AbortController();
    // Desktop webview used to sit on the loading mark forever when serve
    // was dead or half-open — hard-cap so the board always appears.
    const timer = window.setTimeout(() => ac.abort(), 2500);
    fetchSetup(ac.signal)
      .then((s) => {
        if (force || s.needs_setup) {
          setState({
            kind: "ready",
            show: true,
            harnesses: s.harnesses,
            force,
          });
        } else {
          setState({ kind: "ready", show: false });
        }
      })
      .catch(() => {
        // Serve down: still honor ?setup=1 so the operator can see the wizard
        // (install will fail loudly). Otherwise don't block the console.
        if (force) {
          setState({
            kind: "ready",
            show: true,
            force: true,
            harnesses: [
              {
                target: "claude",
                harness: "claude-code",
                path: "~/.claude/settings.json",
                config_exists: false,
                ok: false,
                present: false,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: "harbard install claude",
                blurb: blurbFor("claude"),
              },
              {
                target: "codex",
                harness: "codex",
                path: "~/.codex/hooks.json",
                config_exists: false,
                ok: false,
                present: false,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: "harbard install codex",
                blurb: blurbFor("codex"),
              },
              {
                target: "cursor",
                harness: "cursor",
                path: "~/.cursor/hooks.json",
                config_exists: false,
                ok: false,
                present: false,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: "harbard install cursor",
                blurb: blurbFor("cursor"),
              },
            ],
          });
        } else {
          setState({ kind: "ready", show: false });
        }
      })
      .finally(() => window.clearTimeout(timer));
    return () => {
      window.clearTimeout(timer);
      ac.abort();
    };
  }, [always]);

  if (state.kind === "loading") {
    return (
      <main className="hb-main hb-setup" aria-busy="true">
        <div className="hb-setup-body">
          <p className="hb-setup-note hb-setup-center">{t("opening")}</p>
        </div>
      </main>
    );
  }

  if (state.show) {
    return (
      <SetupScreen
        initial={{ harnesses: state.harnesses }}
        force={state.force}
        onFinished={() => {
          // Drop ?setup=1 so a refresh doesn't reopen the wizard.
          if (typeof window !== "undefined") {
            const u = new URL(window.location.href);
            if (u.searchParams.has("setup")) {
              u.searchParams.delete("setup");
              window.history.replaceState({}, "", u.pathname + u.search + u.hash);
            }
          }
          setState({ kind: "ready", show: false });
        }}
      />
    );
  }

  return children;
}
