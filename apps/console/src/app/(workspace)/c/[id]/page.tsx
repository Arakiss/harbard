import type { Route } from "next";
import { redirect } from "next/navigation";
import { ChannelScreen } from "@/components/workspace/home";
import { LOBBY } from "@/lib/rooms";

export default async function ChannelPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const channel = decodeURIComponent(id);
  // `#lobby` has a screen of its own; an old or typed link still lands there.
  if (channel === LOBBY) redirect("/lobby" as Route);
  return <ChannelScreen channel={channel} />;
}
