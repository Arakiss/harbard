/**
 * Static contract for the dictionaries, for what the type system cannot see:
 * placeholders live inside JSON strings, and copy left in the source compiles
 * fine. `bun run scripts/i18n-audit.ts [path-prefix…]` prints what it finds;
 * `src/i18n/i18n.test.ts` fails on any of it.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { type Feature, en, es } from "../src/i18n/dictionaries";

export type Finding = { file: string; line: number; kind: string; detail: string };

const CONSOLE_ROOT = fileURLToPath(new URL("..", import.meta.url));
const SOURCE_ROOT = join(CONSOLE_ROOT, "src");
const EXEMPT_MARK = "i18n-exempt";
const TRANSLATED_ATTRIBUTES = new Set(["aria-label", "title", "placeholder", "alt", "label", "description", "hint"]);
const PROPER_NOUNS = /\b(harbard|Harbard|Strip|Huginn|macOS|Dock|Claude|Codex|Cursor|Git|GitHub|Tauri|OK)\b/g;
const SPANISH_LETTERS = /[áéíóúñÁÉÍÓÚÑ¿¡«»]/;
const SPANISH_WORDS = /(^|\s)(el|los|las|del|que|para|con|sin|una|está|están|hoy|ayer|ahora|hace|nadie|todavía|aún|también|sesión|agente|agentes|equipo|ajustes)(\s|$|[.,:;])/i;
const FEATURES = Object.keys(en).filter((name): name is Feature => name in en);

function placeholders(template: string): string[] {
  return [...new Set([...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1] ?? ""))].sort();
}

function entries(table: Record<string, string>): [string, string][] {
  return Object.entries(table);
}

export function auditDictionaries(): Finding[] {
  const findings: Finding[] = [];
  for (const feature of FEATURES) {
    const reference: Record<string, string> = en[feature];
    const translated: Record<string, string> = es[feature];
    const report = (key: string, kind: string, detail: string) =>
      findings.push({ file: `dictionaries/*/${feature}.json`, line: 0, kind, detail: `${key}: ${detail}` });
    for (const [key, english] of entries(reference)) {
      const spanish = translated[key];
      if (spanish === undefined) report(key, "missing-es", "no Spanish entry");
      else if (placeholders(english).join() !== placeholders(spanish).join()) {
        report(key, "placeholder-mismatch", `en {${placeholders(english)}} vs es {${placeholders(spanish)}}`);
      }
      if (SPANISH_LETTERS.test(english)) report(key, "spanish-in-en", english);
    }
    for (const key of Object.keys(translated)) if (!(key in reference)) report(key, "extra-es", "not in English");
  }
  return findings;
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) return name === "ui" || name === "i18n" ? [] : sourceFiles(path);
    return /\.tsx?$/.test(name) && !/\.test\.|fixture/.test(name) ? [path] : [];
  });
}

function isFeature(name: string): name is Feature {
  return FEATURES.some((feature) => feature === name);
}

function featureOfCall(node: ts.Expression): Feature | null {
  const call = ts.isAwaitExpression(node) ? node.expression : node;
  if (!ts.isCallExpression(call) || !ts.isIdentifier(call.expression)) return null;
  if (!["createT", "serverT", "translatorFor"].includes(call.expression.text)) return null;
  const name = call.arguments[call.arguments.length - 1];
  return name && ts.isStringLiteral(name) && isFeature(name.text) ? name.text : null;
}

function atModuleScope(node: ts.Node): boolean {
  for (let current = node.parent; current; current = current.parent) if (ts.isFunctionLike(current)) return false;
  return true;
}

function objectKeys(node: ts.Expression | undefined): string[] {
  if (!node || !ts.isObjectLiteralExpression(node)) return [];
  return node.properties.flatMap((property) => (property.name && (ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)) ? [property.name.text] : [])).sort();
}

function isCopyPosition(node: ts.Node): boolean {
  const parent = node.parent;
  if (ts.isImportDeclaration(parent) || ts.isExportDeclaration(parent) || ts.isLiteralTypeNode(parent)) return false;
  if (ts.isExpressionStatement(parent)) return false;
  if (ts.isPropertyAssignment(parent) && parent.name === node) return false;
  return !(ts.isJsxAttribute(parent) && !TRANSLATED_ATTRIBUTES.has(parent.name.getText()));
}

function auditSource(path: string): Finding[] {
  const file = relative(CONSOLE_ROOT, path);
  const text = readFileSync(path, "utf8");
  const lines = text.split("\n");
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, path.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const translators = new Map<string, Feature>();
  const findings: Finding[] = [];
  const report = (node: ts.Node, kind: string, detail: string) => {
    const line = source.getLineAndCharacterOfPosition(node.getStart()).line;
    if (lines[line]?.includes(EXEMPT_MARK) || lines[line - 1]?.includes(EXEMPT_MARK)) return;
    findings.push({ file, line: line + 1, kind, detail: detail.slice(0, 90) });
  };

  const collect = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
      const feature = featureOfCall(node.initializer);
      if (feature) translators.set(node.name.text, feature);
    }
    ts.forEachChild(node, collect);
  };
  collect(source);

  const checkCall = (node: ts.CallExpression, name: string) => {
    if (atModuleScope(node)) report(node, "module-scope", `${name}() would freeze the first language`);
    const feature = translators.get(name);
    const keyNode = node.arguments[0];
    if (!feature || !keyNode || !ts.isStringLiteralLike(keyNode)) return;
    const table: Record<string, string> = en[feature];
    const template = table[keyNode.text];
    if (template === undefined) return report(node, "unknown-key", `${feature}.${keyNode.text}`);
    const wrapper = node.parent;
    const inRich = ts.isCallExpression(wrapper) && ts.isIdentifier(wrapper.expression) && wrapper.expression.text === "richText" && wrapper.arguments[0] === node;
    const given = objectKeys(inRich ? wrapper.arguments[1] : node.arguments[1]);
    if (given.join() !== placeholders(template).join()) {
      report(node, "wrong-vars", `${feature}.${keyNode.text} wants {${placeholders(template)}}, got {${given}}`);
    }
  };

  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const name = node.expression.text;
      if (translators.has(name) || name === "intlTag") checkCall(node, name);
    }
    if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
      for (const argument of node.arguments ?? []) {
        if (ts.isStringLiteralLike(argument) && argument.text === "es-ES") report(argument, "hard-coded-format", "use intlTag()");
      }
    }
    if (ts.isJsxText(node) && /[A-Za-zÀ-ÿ]{2}/.test(node.text.replace(PROPER_NOUNS, ""))) {
      report(node, "jsx-copy", node.text.trim());
    } else if (ts.isStringLiteralLike(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) {
      const value = node.text;
      const copyAttribute = ts.isJsxAttribute(node.parent) && TRANSLATED_ATTRIBUTES.has(node.parent.name.getText());
      if (copyAttribute && /[A-Za-zÀ-ÿ]{2}/.test(value.replace(PROPER_NOUNS, ""))) report(node, "attribute-copy", value);
      else if (isCopyPosition(ts.isStringLiteralLike(node) ? node : node.parent) && (SPANISH_LETTERS.test(value) || SPANISH_WORDS.test(value))) {
        report(node, "spanish-literal", value);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return findings;
}

export function auditAll(prefixes: readonly string[] = []): Finding[] {
  const files = sourceFiles(SOURCE_ROOT).filter((path) => prefixes.length === 0 || prefixes.some((prefix) => relative(CONSOLE_ROOT, path).startsWith(prefix)));
  return [...auditDictionaries(), ...files.flatMap(auditSource)];
}

if (import.meta.main) {
  const findings = auditAll(process.argv.slice(2));
  for (const finding of findings) console.log(`${finding.file}:${finding.line} [${finding.kind}] ${finding.detail}`);
  console.log(`${findings.length} findings`);
  process.exit(findings.length === 0 ? 0 : 1);
}
