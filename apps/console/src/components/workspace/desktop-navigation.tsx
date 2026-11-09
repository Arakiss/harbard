"use client";

import type { Route } from "next";
import * as React from "react";
import { useRouter } from "next/navigation";
import { OPEN_UPDATE_EVENT, SHELL_VIEWS, shellRoute, updateDestination, validMainDestination, type MainDestination } from "@/lib/desktop-view";

type Host = Window & { __HARBARD_PENDING_DESTINATION__?: unknown };

function forShell(value: unknown): value is MainDestination {
  return validMainDestination(value) && (SHELL_VIEWS as readonly string[]).includes(value.view);
}

/**
 * The main window's side of the desktop bridge for the workspace shell: the
 * strip asks the app to open an agent's Direct conversation or an
 * Activity item, the app evaluates `harbard:navigate` here (or leaves the
 * destination pending while the page loads), and the shell goes to that
 * route. The tray's "Check for updates…" and the new-version
 * notification send `{ view: "update" }`, which opens the update dialog
 * (update-state.tsx) instead of a route. Anything else is refused, and the
 * pending global is cleared so it never lingers (the previous console that
 * read the other views is gone).
 */
export function DesktopNavigation() {
  const router = useRouter();
  React.useEffect(() => {
    const host = window as Host;
    const go = (destination: MainDestination) => {
      const route = shellRoute(destination);
      if (route) router.push(route as Route);
    };
    const take = (value: unknown) => {
      if (forShell(value)) go(value);
      else if (updateDestination(value)) window.dispatchEvent(new CustomEvent(OPEN_UPDATE_EVENT));
    };
    const listener = (event: Event) => {
      if (!(event instanceof CustomEvent)) return;
      delete host.__HARBARD_PENDING_DESTINATION__;
      take(event.detail);
    };
    window.addEventListener("harbard:navigate", listener);
    const queued = host.__HARBARD_PENDING_DESTINATION__;
    if (queued !== undefined) delete host.__HARBARD_PENDING_DESTINATION__;
    take(queued);
    return () => window.removeEventListener("harbard:navigate", listener);
  }, [router]);
  return null;
}
