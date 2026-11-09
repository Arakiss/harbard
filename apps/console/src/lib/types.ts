/**
 * Wire types for the frozen `harbard serve` contract (SPEC-API).
 *
 * These mirror the Rust core structs serialized by `serve`. Two fields are held
 * tolerant on purpose: the SPEC's field list names `first_ms` and a `live|stale`
 * liveness, while the core struct as it stands serializes `last_kind` and a third
 * `ended` verdict. The console must work against whichever the real serve emits,
 * so the optional/widened fields below absorb that drift without a client rewrite.
 */

export interface HookHealth {
  target: string;
  harness: string;
  path: string;
  config_exists: boolean;
  ok: boolean;
  missing_events: string[];
  retired_lingering: string[];
  hint: string | null;
  /** Present on GET /setup — binary or config home detected. */
  present?: boolean;
  /** Present && !ok — Setup should pre-select this target. */
  recommended?: boolean;
  /** One-line why this harness matters on the bus. */
  blurb?: string;
}

export interface SetupHarness extends HookHealth {
  present: boolean;
  recommended: boolean;
  blurb: string;
}

export interface SetupStatus {
  needs_setup: boolean;
  completed_ms: number | null;
  skipped: string[];
  wired: string[];
  harnesses: SetupHarness[];
  loopback_install: boolean;
}

export interface SetupInstallResult {
  target: string;
  ok: boolean;
  path?: string;
  events?: string[];
  backup?: string | null;
  error?: string;
}

export interface SetupInstallResponse {
  ok: boolean;
  results: SetupInstallResult[];
  setup: SetupStatus;
}

export interface MetaResponse {
  name: string;
  version: string;
  host: string;
  started_ms: number;
  hub?: boolean;
  bind?: string;
  device?: string;
}

export interface ApiError {
  error: string;
}
