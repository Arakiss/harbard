import { DeletedRoomScreen } from "@/components/workspace/rooms";

export default async function DeletedRoomPage({ params }: { params: Promise<{ name: string }> }) {
  const { name } = await params;
  return <DeletedRoomScreen room={decodeURIComponent(name)} />;
}
