/**
 * Opens the workspace menu from outside the rail: the current workspace's
 * name at the top of the sidebar opens it, under itself. With more than one
 * workspace that name is the only way in, since each rail tile switches
 * workspace; with one, the rail's tile opens the same menu too.
 */
export const WORKSPACE_MENU_EVENT = "hb:workspace-menu";

export interface MenuAnchor {
  left: number;
  bottom: number;
}

export function openWorkspaceMenu(anchor: MenuAnchor): void {
  window.dispatchEvent(new CustomEvent<MenuAnchor>(WORKSPACE_MENU_EVENT, { detail: anchor }));
}
