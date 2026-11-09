import { redirect } from "next/navigation";

/** Team was removed: teammates and subagents are Home's Agents now. */
export default function TeamPage() {
  redirect("/");
}
