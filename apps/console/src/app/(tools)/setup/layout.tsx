import { lato } from "@/components/workspace/lato";
import { MoreFrame } from "@/components/workspace/shell";
import { ThemeProvider } from "@/components/workspace/theme-context";
import { fetchInitialTheme } from "@/lib/initial-theme";
import "@/components/workspace/workspace.css";
import "@/components/robot/robot.css";
import "@/components/brand/mark.css";

// A tool under "More tools": the same rail as the shell, so Home is
// one click away. The rail follows the theme; the wizard keeps its own look.
export default async function ToolsLayout({ children }: { children: React.ReactNode }) {
  const theme = await fetchInitialTheme();
  return (
    <div className={lato.variable}>
      <ThemeProvider initial={theme}>
        <MoreFrame>{children}</MoreFrame>
      </ThemeProvider>
    </div>
  );
}
