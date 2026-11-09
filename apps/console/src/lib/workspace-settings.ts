/**
 * What "<workspace> settings" shows about each repository: its name and why
 * it sits in this workspace, from `GET /api/v1/workspaces`.
 */
import { createT } from "@/i18n";
import type { WorkspacesList } from "./api";

const t = createT("shell");

type Placed = WorkspacesList["repos"][number];

/** The repositories placed in workspace `id`, by name. */
export function reposIn(list: Pick<WorkspacesList, "repos">, id: string): Placed[] {
  return list.repos.filter((r) => r.workspace === id).sort((a, b) => repoName(a.repo).localeCompare(repoName(b.repo)));
}

/** A repository's last path segment. */
export function repoName(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/** Why it is here, in the operator's words. */
export function placementLabel(r: Pick<Placed, "moved" | "reason">): string {
  if (r.moved) return t("placementMoved");
  const owner = r.reason.match(/^origin owner (.+)$/)?.[1];
  if (owner) return t("placementOriginOwner", { owner });
  if (r.reason.startsWith("paired device ")) return t("placementPairedDevice");
  return t("placementDefault");
}
