import type { Metadata } from "next";
import { DesktopNavigation } from "@/components/workspace/desktop-navigation";
import { lato } from "@/components/workspace/lato";
import { Shell } from "@/components/workspace/shell";
import { ThemeProvider } from "@/components/workspace/theme-context";
import { WorkspaceProvider } from "@/components/workspace/workspace-context";
import { fetchInitialTheme } from "@/lib/initial-theme";
import { fetchInitialWorkspace } from "@/lib/initial-workspace";
import "@/components/workspace/workspace.css";
import "@/components/robot/robot.css";
import "@/components/workspace/direct.css";
import "@/components/workspace/rooms.css";
import "@/components/brand/mark.css";
import "@/components/feedback/feedback.css";
import "@/components/retro/retro.css";
import "@/components/huginn/huginn.css";

export const metadata: Metadata = {
  title: "harbard",
  robots: { index: false, follow: false },
};

export const dynamic = "force-dynamic";

// No first-run gate here: the shell reads transcripts and the bus whether or
// not hooks are wired, and it renders on the server with its data. The setup
// wizard is "Set up harbard" under "More tools" (`/setup`;
// `/?setup=1` forwards there).

// The stored theme is in the first HTML (a <style> and the root's
// data-hb-theme/-mode), so a custom theme never flashes Aubergine first.
export default async function WorkspaceLayout({ children }: { children: React.ReactNode }) {
  const [initial, theme] = await Promise.all([fetchInitialWorkspace(), fetchInitialTheme()]);
  return (
    <div className={lato.variable}>
      <ThemeProvider initial={theme}>
        <WorkspaceProvider initial={initial} hiddenDesktopPoll>
          {/* FeedbackProvider lives in the Shell, inside the profile: feedback goes quiet in Focused, Swamped and Away. */}
          <Shell>{children}</Shell>
          <DesktopNavigation />
        </WorkspaceProvider>
      </ThemeProvider>
    </div>
  );
}
