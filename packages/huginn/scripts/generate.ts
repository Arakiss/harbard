/** Writes what the Rust CLI embeds. `bun run generate` after a catalog change. */

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { agentGuide, wireCatalog } from "../src/wire";

const out = join(import.meta.dir, "..", "..", "..", "crates", "harbard-cli", "src", "huginn");
writeFileSync(join(out, "catalog.json"), `${JSON.stringify(wireCatalog(), null, 1)}\n`);
writeFileSync(join(out, "guide.md"), agentGuide());
