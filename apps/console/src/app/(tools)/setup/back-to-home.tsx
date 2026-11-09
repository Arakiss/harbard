"use client";

import * as React from "react";
import { useRouter } from "next/navigation";

/** Once the wizard is done, back to Home. */
export function BackToHome() {
  const router = useRouter();
  React.useEffect(() => router.replace("/"), [router]);
  return null;
}
