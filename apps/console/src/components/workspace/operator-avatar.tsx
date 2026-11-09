"use client";

import * as React from "react";
import { BellOff } from "lucide-react";
import { states, type StateId } from "@/lib/profile";
import { StateIcon } from "./state-icon";
import { Avatar } from "./parts";
import { useProfile } from "./profile-state";
import { avatarPhotoUrl, defaultAvatarId } from "@/lib/avatar-gallery";

/**
 * The operator's face: the photo or gallery avatar they chose, else a default
 * gallery avatar picked by name; initials only if that image fails to load.
 */
export function OperatorAvatar({
  name,
  initials,
  size,
  presence,
}: {
  name: string;
  initials: string;
  size: number;
  /** The state's mark; null draws none (the big avatar in Profile). */
  presence: StateId | "do-not-disturb" | null;
}) {
  const profile = useProfile();
  const [broken, setBroken] = React.useState<string | null>(null);
  const url = profile?.photoUrl ?? avatarPhotoUrl(defaultAvatarId(name));
  const radius = Math.max(6, Math.round(size * 0.22));
  return (
    <span className="hb-op-av" style={{ width: size, height: size }} data-presence={presence ?? undefined}>
      {url && broken !== url ? (
        <img src={url} alt="" className="hb-op-photo" style={{ borderRadius: radius }} onError={() => setBroken(url)} />
      ) : (
        <Avatar name={name} size={size} colour="var(--hb-av-operator)" initials={initials} />
      )}
      {presence === "available" ? <span className="hb-av-dot" data-tone="green" /> : null}
      {presence === "do-not-disturb" ? (
        <span className="hb-op-badge" data-tone="grey">
          <BellOff className="size-[10px]" strokeWidth={2.6} aria-hidden />
        </span>
      ) : null}
      {presence && presence !== "available" && presence !== "do-not-disturb" ? (
        <span className="hb-op-badge" data-tone={states().find((s) => s.id === presence)?.tone}>
          <StateIcon id={presence} className="size-[10px]" />
        </span>
      ) : null}
    </span>
  );
}
