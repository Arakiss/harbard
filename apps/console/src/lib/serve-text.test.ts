import { afterEach, describe, expect, test } from "bun:test";
import { setActiveLocale } from "@/i18n";
import { DEFAULT_LOCALE } from "@/i18n/locale";
import { readableText } from "./legible";
import { lessonEvidence } from "./retro";
import { serveText } from "./serve-text";
import { type ThemeRefusal, type ThemeState, describeRefusal, themeNotice } from "./theme";
import { type ReadablePart, type WsAgent, agentName, agentRoleLine, shownName } from "./workspace";

afterEach(() => setActiveLocale(DEFAULT_LOCALE));

function inBothLanguages(render: () => string): [string, string] {
  setActiveLocale("en");
  const english = render();
  setActiveLocale("es");
  const spanish = render();
  return [english, spanish];
}

describe("a sentence from serve", () => {
  test("an unknown code shows serve's English", () => {
    const [en, es] = inBothLanguages(() => serveText("theme.from_a_later_build", { name: "x" }, "serve's words"));
    expect(en).toBe("serve's words");
    expect(es).toBe("serve's words");
  });

  test("no code shows serve's text", () => {
    expect(serveText(undefined, undefined, "as written")).toBe("as written");
  });
});

describe("theme refusals", () => {
  const unknownKey: ThemeRefusal = {
    file: "/t/x.toml",
    line: 3,
    key: "sidebar_hover",
    code: "unknown_key",
    message: "`sidebar_hover` is not a theme key",
    fix: "did you mean `sidebar-hover`? The valid keys are in _example.toml",
    message_code: "theme.unknown_key",
    fix_code: "theme.unknown_key.fix_suggest",
    params: { key: "sidebar_hover", suggestion: "sidebar-hover", example: "_example.toml" },
  };

  test("word the message and the fix from the code", () => {
    const [en, es] = inBothLanguages(() => describeRefusal(unknownKey));
    expect(en).toBe("/t/x.toml:3: `sidebar_hover`: `sidebar_hover` is not a theme key. Fix: did you mean `sidebar-hover`? The valid keys are in _example.toml");
    expect(es).toContain("`sidebar_hover` no es una clave de tema");
    expect(es).toContain("¿querías decir `sidebar-hover`? Las claves válidas están en _example.toml");
  });

  test("word a low contrast pair by its key", () => {
    const refusal: ThemeRefusal = {
      file: "/t/x.toml",
      code: "low_contrast",
      message: "insufficient contrast in dark mode: [dark] `accent-ink` on `accent` (selected channel) gives 2.10:1 and needs 4.5:1",
      fix: "change `accent-ink` so it reads over `accent`, or remove it and harbard derives it",
      message_code: "theme.low_contrast.dark",
      fix_code: "theme.low_contrast.fix_fg",
      params: { section: "[dark] ", fg: "accent-ink", on: "accent", first_bg: "accent", what: "selected channel", what_key: "selected_channel", ratio: "2.10", min: "4.5" },
    };
    const [en, es] = inBothLanguages(() => describeRefusal(refusal));
    expect(en).toContain("(selected channel) gives 2.10:1 and needs 4.5:1");
    expect(es).toContain("contraste insuficiente en modo oscuro: [dark] `accent-ink` sobre `accent` (canal seleccionado) da 2.10:1 y hace falta 4.5:1");
    expect(es).toContain("cambia `accent-ink` para que se lea sobre `accent`");
  });

  test("name a pasted share code in the operator's language", () => {
    const refusal: ThemeRefusal = {
      file: "share code",
      code: "bad_code",
      message: "the theme name is missing",
      fix: "copy the whole code",
      message_code: "theme.bad_code.name",
      fix_code: "theme.bad_code.fix",
    };
    const [en, es] = inBothLanguages(() => describeRefusal(refusal));
    expect(en).toStartWith("share code: the theme name is missing.");
    expect(es).toStartWith("código compartido: falta el nombre del tema.");
  });

  test("keep serve's text for a refusal with no code", () => {
    const [, es] = inBothLanguages(() => describeRefusal({ file: "f", code: "exists", message: "as sent", fix: "keep it" }));
    expect(es).toContain("as sent");
    expect(es).toContain("keep it");
  });
});

describe("the theme notice", () => {
  const base: Omit<ThemeState, "notice" | "notice_code" | "notice_params" | "errors"> = {
    version: 1,
    revision: "r",
    theme: "ink",
    mode: "light",
    stored: { theme: "mio", mode: null },
    themes: [],
    dir: "/t",
    modes: { light: {}, dark: {} },
  };

  test("quotes the refusal in the operator's language", () => {
    const state: ThemeState = {
      ...base,
      errors: [{ file: "/t/mio.toml", code: "bad_colour", message: "`#12` is not a hexadecimal colour", fix: "f", message_code: "theme.bad_colour.hex", fix_code: "theme.bad_colour.hex.fix", params: { value: "#12" } }],
      notice: 'The theme "mio" cannot be applied (`#12` is not a hexadecimal colour); using the default theme.',
      notice_code: "theme.notice.refused",
      notice_params: { theme: "mio", why: "`#12` is not a hexadecimal colour", refusal: 0 },
    };
    const [en, es] = inBothLanguages(() => themeNotice(state) ?? "");
    expect(en).toBe(state.notice ?? "");
    expect(es).toBe("El tema «mio» no se puede aplicar (`#12` no es un color hexadecimal); se usa el tema por defecto.");
  });

  test("names a stored mode that does not exist", () => {
    const state: ThemeState = { ...base, errors: [], notice: 'The saved mode "noche" does not exist; using Light.', notice_code: "theme.notice.bad_mode", notice_params: { mode: "noche" } };
    const [en, es] = inBothLanguages(() => themeNotice(state) ?? "");
    expect(en).toBe(state.notice ?? "");
    expect(es).toBe("El modo guardado «noche» no existe; se usa Claro.");
  });

  test("is absent when serve sends none", () => {
    expect(themeNotice({ ...base, errors: [] })).toBeNull();
  });
});

describe("a lesson's evidence", () => {
  test("words stalls with one noun per language", () => {
    const lesson = { evidence: "3 stalls waiting for a permission, the longest 60 min (2026-11-07).", evidence_code: "retro.evidence.stalls", evidence_params: { n_stall: 3, longest: 60, dates: "2026-11-07" } };
    const [en, es] = inBothLanguages(() => lessonEvidence(lesson));
    expect(en).toBe(lesson.evidence);
    expect(es).toBe("3 paradas esperando un permiso, la más larga de 60 min (2026-11-07).");
  });

  test("words repeated findings clause by clause", () => {
    const lesson = {
      evidence: "3 secret leaks on 2 branches (a, b); 1 file-read race on 1 branch (c). The same piece was written on each branch.",
      evidence_code: "retro.evidence.shared_security",
      evidence_params: { items: 2, class_0: "secret_leak", count_0: 3, branches_0: 2, names_0: "a, b", class_1: "file_race", count_1: 1, branches_1: 1, names_1: "c" },
    };
    const [en, es] = inBothLanguages(() => lessonEvidence(lesson));
    expect(en).toBe(lesson.evidence);
    expect(es).toBe("3 fugas de secretos en 2 ramas (a, b); 1 carrera al leer ficheros en 1 rama (c). La misma pieza se escribió en cada rama.");
  });

  test("agrees a single background agent's verb", () => {
    const lesson = { evidence: "1 agent ended its turn waiting for a background task.", evidence_code: "retro.evidence.background_wait.one", evidence_params: { count: 1 } };
    const [en, es] = inBothLanguages(() => lessonEvidence(lesson));
    expect(en).toBe(lesson.evidence);
    expect(es).toBe("1 agente terminó el turno esperando una tarea en segundo plano.");
  });
});

describe("a readable line", () => {
  const delivered: ReadablePart[] = [
    { kind: "text", text: "Reviewer: " },
    { kind: "text", text: "CAP-1 and CAP-2 done and pushed", code: "readable.done.pushed.many", params: { who: "CAP-1 and CAP-2", head: "CAP-1", last: "CAP-2" } },
    { kind: "text", text: " · 82 of 82 tests pass", code: "readable.tests.count", params: { passed: 82, total: 82 } },
  ];

  test("words each coded piece and keeps the rest", () => {
    const [en, es] = inBothLanguages(() => readableText(delivered));
    expect(en).toBe("Reviewer: CAP-1 and CAP-2 done and pushed · 82 of 82 tests pass");
    expect(es).toBe("Reviewer: CAP-1 y CAP-2 terminados y subidos · 82 de 82 pruebas pasan");
  });

  test("agrees a single subject", () => {
    const [, es] = inBothLanguages(() => readableText([{ kind: "text", text: "CAP-9 approved", code: "readable.approved", params: { who: "CAP-9", head: "", last: "CAP-9" } }]));
    expect(es).toBe("CAP-9 aprobado");
  });
});

describe("a generated agent name", () => {
  const coded = (name: string, name_code: string, name_params: Record<string, string>): Pick<WsAgent, "name" | "name_code" | "name_params"> => ({ name, name_code, name_params });

  test("words a known role with its number", () => {
    const [en, es] = inBothLanguages(() => agentName(coded("keychain · reviewer 2", "name.team_role", { team: "keychain", role: "reviewer", number: "2" })));
    expect(en).toBe("keychain · reviewer 2");
    expect(es).toBe("keychain · revisor 2");
  });

  test("words an unnamed session", () => {
    const [en, es] = inBothLanguages(() => agentName(coded("harbard · session 17:26", "name.session.project_time", { project: "harbard", time: "17:26" })));
    expect(en).toBe("harbard · session 17:26");
    expect(es).toBe("harbard · sesión 17:26");
  });

  test("leaves a chosen name as it was written", () => {
    const [, es] = inBothLanguages(() => agentName({ name: "Nora" }));
    expect(es).toBe("Nora");
  });

  test("words a message author known only by its session id", () => {
    const [en, es] = inBothLanguages(() => shownName("session ab12cd34"));
    expect(en).toBe("session ab12cd34");
    expect(es).toBe("sesión ab12cd34");
  });
});

describe("a role line", () => {
  test("agrees a known role with the name's gender", () => {
    const line = { role_line: "reviewer · protocol", role_line_code: "role_line", role_line_params: { role: "reviewer", gender: "f", team: "protocol" } };
    const [en, es] = inBothLanguages(() => agentRoleLine(line));
    expect(en).toBe("reviewer · protocol");
    expect(es).toBe("revisora · protocol");
  });

  test("words a role with no team", () => {
    const [en, es] = inBothLanguages(() => agentRoleLine({ role_line: "builder", role_line_code: "role_line.solo", role_line_params: { role: "builder", gender: "m" } }));
    expect(en).toBe("builder");
    expect(es).toBe("constructor");
  });

  test("leaves a line someone wrote as it is", () => {
    const [, es] = inBothLanguages(() => agentRoleLine({ role_line: "the ledger · x" }));
    expect(es).toBe("the ledger · x");
  });
});
