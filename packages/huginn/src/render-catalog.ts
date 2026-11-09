/**
 * json-render's catalog over the same definitions, for its `Renderer`.
 * Kept apart from `catalog.ts` so the Rust generator and the validator do not
 * load json-render.
 */

import { defineCatalog } from "@json-render/core";
import { schema } from "@json-render/react/schema";
import { COMPONENTS } from "./catalog";

export const huginnCatalog = defineCatalog(schema, {
  components: Object.fromEntries(
    Object.entries(COMPONENTS).map(([name, definition]) => [name, { props: definition.props, description: definition.description }]),
  ),
  actions: {},
});
