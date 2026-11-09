import { createT } from "@/i18n";

const t = createT("messages");

/**
 * serve refuses text that carries characters nobody sees but a model reads
 * (format and unassigned code points: zero-width marks, bidi controls, tag
 * characters), naming them as `U+XXXX`. This is that refusal in
 * the console's language, with the code points, or `null` for any other message.
 */
export function invisibleRefusal(message: string): string | null {
  if (!message.includes("characters you cannot see")) return null;
  const found = message.match(/U\+[0-9A-F]{4,6}/g) ?? [];
  const list = found.length > 0 ? ` (${found.join(", ")})` : "";
  return t("invisibleRefusal", { list });
}
