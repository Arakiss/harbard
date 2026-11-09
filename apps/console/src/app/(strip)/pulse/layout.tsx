import type { Metadata } from "next";
import { headers } from "next/headers";
import { lato } from "@/components/workspace/lato";
import { ThemeProvider } from "@/components/workspace/theme-context";
import { WorkspaceProvider } from "@/components/workspace/workspace-context";
import { fetchInitialTheme } from "@/lib/initial-theme";
import { ALL_WORKSPACES } from "@/lib/current-workspace";
import { fetchInitialWorkspace } from "@/lib/initial-workspace";
import "@/components/workspace/workspace.css";
import "@/components/robot/robot.css";
import "@/components/brand/mark.css";
import "@/components/strip/strip.css";
import { STAGE_FIT_SCRIPT as FIT } from "@/components/strip/model";

export const metadata: Metadata = {
  title: "harbard · Strip",
  robots: { index: false, follow: false },
};

export const dynamic = "force-dynamic";

/**
 * The strip is drawn on a 2560×720 stage and zoomed to the window before the
 * first paint, so the desktop shell's zoom, a smaller fallback window and a
 * 1x or 2x screenshot all show the same composition: `stripFit` in
 * components/strip/model.ts, which the strip applies again on resize.
 * The console's CSP only runs inline scripts that carry the request's
 * nonce; without it this script was blocked and the stage stayed at 2560
 * wide, cropped in any smaller window.
 */

// The strip (a 2560×720 touch display): the workspace view and
// the live faces, in the first HTML like the shell, without its rail.
export default async function StripLayout({ children }: { children: React.ReactNode }) {
  const [initial, theme, h] = await Promise.all([fetchInitialWorkspace(ALL_WORKSPACES), fetchInitialTheme(), headers()]);
  const nonce = h.get("x-nonce") ?? undefined;
  return (
    <div className={lato.variable}>
      <script nonce={nonce} dangerouslySetInnerHTML={{ __html: FIT }} />
      <ThemeProvider initial={theme}>
        <WorkspaceProvider initial={initial}>{children}</WorkspaceProvider>
      </ThemeProvider>
    </div>
  );
}
