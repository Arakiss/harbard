import type * as React from "react";
import type { LucideIcon } from "lucide-react";

/** A Preferences setting's title with its icon, so a page can be scanned by shape before it is read. */
export function PrefsLegend({ icon: Icon, children }: { icon: LucideIcon; children: React.ReactNode }) {
  return (
    <legend className="hb-prefs-legend">
      <Icon className="hb-prefs-legend-icon" aria-hidden />
      {children}
    </legend>
  );
}
