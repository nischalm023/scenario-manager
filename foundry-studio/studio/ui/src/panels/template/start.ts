// Spec 05 section 4.2: "Use this template" starts a draft in the person's New site conversation, which the API opens
// or answers (spec 01 section 3.17); until the agent loop posts the user entry that starts a turn (task 3), the
// browser then opens the draft's panel itself (spec 02's rule F6; lead decided).

import type { OpenedConversation } from '../../protocol/conversation.ts';
import type { DraftStarted } from '../../protocol/template_draft.ts';
import { postJson, type Answer } from '../../shell/api.ts';
import { sendJson } from './requests.ts';

export async function startDraft(template: string, browserTab: string): Promise<Answer<DraftStarted>> {
  const opened = await postJson<OpenedConversation>('/api/conversations', {});
  const conversation = opened.ok ? { conversation: opened.value.conversation.id } : {};
  return sendJson<DraftStarted>('POST', '/api/template-drafts', { template: `${template}@main`, ...conversation, browser_tab: browserTab });
}
