import { DirectsScreen } from "@/components/workspace/direct-messages";

export default async function DirectPage({ params }: { params: Promise<{ agent: string }> }) {
  const { agent } = await params;
  return <DirectsScreen agent={decodeURIComponent(agent)} />;
}
