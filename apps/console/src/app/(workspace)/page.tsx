import type { Route } from "next";
import { redirect } from "next/navigation";
import { ChannelScreen } from "@/components/workspace/home";
import { legacyRedirect } from "@/lib/workspace";

export const dynamic = "force-dynamic";

type Search = Promise<Record<string, string | string[] | undefined>>;

/**
 * Home. Deep links into the removed previous console (`/?view=inbox&note=…`,
 * `/?setup=1`) land where their content lives now (`legacyRedirect`).
 */
export default async function HomePage({ searchParams }: { searchParams: Search }) {
  const to = legacyRedirect(await searchParams);
  if (to) redirect(to as Route);
  return <ChannelScreen channel={null} />;
}
