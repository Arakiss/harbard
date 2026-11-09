import type * as React from "react";
import { Code2, FileText, GitBranch, Globe, Search, Send, Settings, Sparkles, SquareTerminal, Users } from "lucide-react";
import type { StepKind } from "@/lib/progress";

export type Icon = React.ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>;

/** The sketches' icons: one per kind of step, in a tinted square. */
export const STEP_ICON: Record<StepKind, Icon> = {
  editing: Code2,
  reading: FileText,
  searching: Search,
  running: Settings,
  testing: SquareTerminal,
  commit: GitBranch,
  web: Globe,
  delegating: Users,
  message: Send,
  thinking: Sparkles,
};
