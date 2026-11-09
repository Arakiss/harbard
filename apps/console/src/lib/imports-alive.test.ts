import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

// The e2e mock, the scripts and the specs import from src, so pruning src as
// dead code can break them without any unit test failing.
const root = join(import.meta.dir, "..", "..");

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "node_modules" ? [] : files(path);
    return /\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe("what lives outside src still finds what it imports", () => {
  test("every relative import from mock, scripts and e2e resolves", () => {
    const missing: string[] = [];
    for (const dir of ["mock", "scripts", "e2e"]) {
      for (const file of files(join(root, dir))) {
        for (const [, spec] of readFileSync(file, "utf8").matchAll(/^\s*(?:import|export)\b[^\n]*?from\s+["'](\.{1,2}\/[^"']+)["']/gm)) {
          const base = resolve(dirname(file), spec!);
          if (![base, `${base}.ts`, `${base}.tsx`, join(base, "index.ts")].some((p) => existsSync(p)))
            missing.push(`${relative(root, file)} → ${spec}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });
});
