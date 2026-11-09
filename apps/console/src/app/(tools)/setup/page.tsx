import type { Metadata } from "next";
import { SetupGate } from "@/components/setup/setup-screen";
import { serverT } from "@/i18n/server";
import { BackToHome } from "./back-to-home";

export const dynamic = "force-dynamic";

export async function generateMetadata(): Promise<Metadata> {
  const t = await serverT("setup");
  return {
    title: t("pageTitle"),
    robots: { index: false, follow: false },
  };
}

/**
 * "Set up harbard": the first-run wizard that wires Claude Code, Codex and
 * Cursor to harbard, always shown here.
 * When it finishes, Home.
 */
export default function SetupPage() {
  return (
    <SetupGate always>
      <BackToHome />
    </SetupGate>
  );
}
