import { EMBLEM_SIZE, emblemOf } from "@/lib/workspace-emblem";

/** A workspace's generated emblem. Decorative: its tile carries the name. */
export function WorkspaceEmblem({ id, palette }: { id: string; palette?: number }) {
  const emblem = emblemOf(id, palette);
  return (
    <svg className="hb-ws-emblem" viewBox={`0 0 ${EMBLEM_SIZE} ${EMBLEM_SIZE}`} aria-hidden>
      <rect width={EMBLEM_SIZE} height={EMBLEM_SIZE} fill={emblem.palette.ground} />
      {emblem.shapes.map((s, i) => (
        <path key={i} d={s.d} transform={s.transform} fill={s.color} />
      ))}
    </svg>
  );
}
