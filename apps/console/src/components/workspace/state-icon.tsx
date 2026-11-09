"use client";

import { Circle, Flag, Flame, Focus, Moon, ScanSearch, type LucideIcon } from "lucide-react";
import type { StateId } from "@/lib/profile";

/** One icon per coordination state. */
const STATE_ICON: Record<StateId, LucideIcon> = {
  available: Circle,
  focused: Focus,
  swamped: Flame,
  away: Moon,
  reviewing: ScanSearch,
  done: Flag,
};

export function StateIcon({ id, className }: { id: StateId; className?: string }) {
  const Icon = STATE_ICON[id];
  return <Icon className={className} strokeWidth={2.2} aria-hidden />;
}
