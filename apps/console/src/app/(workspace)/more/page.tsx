import type { Route } from "next";
import { redirect } from "next/navigation";
import { moreRedirect } from "@/lib/workspace";

export const dynamic = "force-dynamic";

type Search = Promise<Record<string, string | string[] | undefined>>;

/**
 * The previous console lived here and was removed. An old bookmark lands
 * where its view lives now: Activity for the inbox (its note selected), the
 * wizard for `?setup=1`, Home for the rest.
 */
export default async function MorePage({ searchParams }: { searchParams: Search }) {
  redirect(moreRedirect(await searchParams) as Route);
}
