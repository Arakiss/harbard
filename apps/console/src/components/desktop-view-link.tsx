"use client";

import { useRef, useState, type ComponentProps } from "react";
import { toast } from "sonner";
import { createT } from "@/i18n";
import { setCurrentWorkspace } from "@/lib/current-workspace";
import {
  desktopViewBridge,
  desktopViewHref,
  openDesktopView,
  type DesktopView,
  type MainDestination,
} from "@/lib/desktop-view";

const t = createT("desktop");

type Props = Omit<ComponentProps<"a">, "href" | "target" | "onClick"> & {
  target: DesktopView;
  destination?: MainDestination;
  /** The workspace the destination lives in: the main window moves to it. */
  workspace?: string;
};

/** A normal web link outside the shell; an existing-window switch inside it. */
export function DesktopViewLink({ target, destination, workspace, children, ...props }: Props) {
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  return (
    <a
      {...props}
      href={desktopViewHref(target, destination)}
      aria-busy={pending || undefined}
      onClick={async (event) => {
        if (workspace) setCurrentWorkspace(workspace);
        const bridge = desktopViewBridge();
        if (bridge.kind === "browser") return;
        event.preventDefault();
        if (inFlight.current) return;
        inFlight.current = true;
        setPending(true);
        try {
          await openDesktopView(target, bridge, destination);
        } catch (error) {
          toast.error(
            error instanceof Error
              ? error.message
              : t("couldNotSwitchViews"),
          );
        } finally {
          inFlight.current = false;
          setPending(false);
        }
      }}
    >
      {children}
    </a>
  );
}
