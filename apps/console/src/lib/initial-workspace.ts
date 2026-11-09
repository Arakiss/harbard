/* Server-only: imported by the workspace layout, never by client components. */
import "server-only";
import { cookies } from "next/headers";
import { harbardAuthHeaders, harbardUpstreamUrl } from "./upstream";
import { WORKSPACE_COOKIE, validWorkspaceId } from "./current-workspace";
import type { WorkspaceView } from "./workspace";
import { consoleCommit } from "./deliveries";

/**
 * The workspace in the first HTML, like the root layout's snapshot: the shell
 * opens on its channels instead of a loading line. Bounded by one timeout; a
 * slow or dead serve degrades to `undefined` and the client fetches as usual.
 */
export async function fetchInitialWorkspace(scope?: string): Promise<WorkspaceView | undefined> {
  try {
    const commit = consoleCommit();
    // The workspace the operator chose last, from its cookie.
    const ws = scope ?? (await cookies()).get(WORKSPACE_COOKIE)?.value;
    const query = [commit ? `console=${commit}` : "", validWorkspaceId(ws) ? `ws=${ws}` : ""].filter(Boolean).join("&");
    const res = await fetch(harbardUpstreamUrl("workspace", query), {
      headers: harbardAuthHeaders({ accept: "application/json" }),
      cache: "no-store",
      signal: AbortSignal.timeout(2500),
    });
    if (!res.ok) return undefined;
    return (await res.json()) as WorkspaceView;
  } catch {
    return undefined;
  }
}
