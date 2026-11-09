import { expect, MOCK_BASE, test } from "./fixtures";

test("thread reads full bodies and transitive replies in append order from any member", async ({ request, mock }) => {
  await mock.reset("empty");
  const append = async (event: Record<string, unknown>) => {
    const response = await request.post(`${MOCK_BASE}/__control/append`, { data: { session: "thread-author", harness: "codex", cwd: "/work/thread-project", repo: "/work/thread-project", ...event } });
    expect(response.ok()).toBeTruthy();
    return (await response.json()).id as string;
  };
  const body = "Keep the complete request, its evidence and the requested follow-up together.\n".repeat(18);
  const root = await append({ event: "note", kind: "handoff", text: body });
  const first = await append({ event: "note", ref: root, text: "First response" });
  const second = await append({ event: "note", ref: root, text: "Second response" });
  const child = await append({ event: "note", ref: first, text: "Follow-up to the first response" });
  const receipt = await append({ event: "ack", text: `${root} — received, not completed` });
  await append({ event: "note", text: "Unrelated message" });
  await append({ event: "ack", text: `${root}extra — not a valid receipt target` });
  for (const id of [root, child]) {
    const response = await request.get(`${MOCK_BASE}/api/v1/notes/${id}/thread`);
    expect(response.status()).toBe(200);
    const thread = await response.json();
    expect(thread.thread).toBe(root);
    expect(thread.count).toBe(5);
    expect(thread.messages.map((message: { id: string }) => message.id)).toEqual([root, first, second, child, receipt]);
    expect(thread.messages[0].text).toBe(body);
    expect(thread.messages[3]).toMatchObject({ text: "Follow-up to the first response", ref: first });
    expect(thread.messages[4]).toMatchObject({ event: "ack", text: `${root} — received, not completed` });
  }
  const missing = await request.get(`${MOCK_BASE}/api/v1/notes/01ARZ3NDEKTSV4RRFFQ69G5FAV/thread`);
  expect(missing.status()).toBe(404);
  expect(await missing.json()).toMatchObject({ error: "unknown_note" });
});
