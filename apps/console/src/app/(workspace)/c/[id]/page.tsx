import { ChannelScreen } from "@/components/workspace/home";

export default async function ChannelPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  return <ChannelScreen channel={decodeURIComponent(id)} />;
}
