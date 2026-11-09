import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { huginnCatalog } from "./render-catalog";
import { COMPONENT_NAMES } from "./catalog";
import { agentGuide, wireCatalog } from "./wire";

const out = join(import.meta.dir, "..", "..", "..", "crates", "harbard-cli", "src", "huginn");

/** The JSON Schema keywords the Rust validator implements; anything else must fail here first. */
const UNDERSTOOD = new Set([
  "type", "properties", "required", "additionalProperties", "items", "enum", "const", "anyOf",
  "minLength", "maxLength", "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minItems", "maxItems", "x-huginn-format",
]);

function keywords(schema: unknown, found: Set<string>): void {
  if (Array.isArray(schema)) return schema.forEach((item) => keywords(item, found));
  if (typeof schema !== "object" || schema === null) return;
  for (const [key, value] of Object.entries(schema)) {
    found.add(key);
    if (key === "properties") Object.values(value as object).forEach((prop) => keywords(prop, found));
    else if (key === "items" || key === "anyOf") keywords(value, found);
  }
}

describe("what the Rust CLI embeds", () => {
  test("catalog.json is the catalog as it stands; run bun run generate after a change", () => {
    expect(JSON.parse(readFileSync(join(out, "catalog.json"), "utf8"))).toEqual(JSON.parse(JSON.stringify(wireCatalog())));
  });

  test("guide.md is the guide as it stands", () => {
    expect(readFileSync(join(out, "guide.md"), "utf8")).toBe(agentGuide());
  });

  test("every schema keyword is one the Rust validator understands", () => {
    const found = new Set<string>();
    const wire = wireCatalog() as { components: Record<string, { props: unknown }> };
    Object.values(wire.components).forEach((component) => keywords(component.props, found));
    expect([...found].filter((key) => !UNDERSTOOD.has(key))).toEqual([]);
  });

  test("every format survives into the schema, so Rust can apply it", () => {
    const wire = JSON.stringify(wireCatalog());
    for (const format of ["http_url", "file_name", "iso_date", "answer_id", "state_path"]) expect(wire).toContain(`"x-huginn-format":"${format}"`);
  });
});

describe("the catalog json-render reads", () => {
  test("holds every component and no action", () => {
    const data = huginnCatalog.data as { components: object; actions: object };
    expect(Object.keys(data.components).sort()).toEqual([...COMPONENT_NAMES].sort());
    expect(Object.keys(data.actions)).toEqual([]);
  });
});
