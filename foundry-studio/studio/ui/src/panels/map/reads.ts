// The Map panel's reads (spec 03 sections 3.3, 3.5 and 3.6): the summary of the site at the address's cut, references
// resolved into marks at that cut, Find, and the agent's highlight entry the address names by its number in the
// site's one journal sequence, read from whichever of the site's conversations holds it.

import type { EntriesPage } from '../../protocol/snapshot.ts';
import type { ConversationList } from '../../protocol/conversation.ts';
import type { FindResults, MapSummary, Resolution, RefMarks } from '../../protocol/map.ts';
import type { PanelCommandEntry } from '../../protocol/panel.ts';
import { getJson, postJson, type Answer } from '../../shell/api.ts';

const kinQuery = (kin: string | undefined): string => (kin ? `kin=${encodeURIComponent(kin)}` : '');

export const readSummary = (location: string, kin?: string): Promise<Answer<MapSummary>> => {
  const query = kinQuery(kin);
  return getJson<MapSummary>(`/api/map/${location}${query ? `?${query}` : ''}`);
};

export const findOnMap = (location: string, kin: string | undefined, words: string): Promise<Answer<FindResults>> => {
  const query = [kinQuery(kin), `q=${encodeURIComponent(words)}`].filter((part) => part !== '').join('&');
  return getJson<FindResults>(`/api/find/${location}?${query}`);
};

/** Each reference resolved on the Map panel at `location`, in the order asked. */
export async function resolveOnMap(location: string, kin: string | undefined, refs: string[]): Promise<Answer<RefMarks[]>> {
  const body = { location, panel: 'map', refs, ...(kin ? { kin } : {}) };
  const answer = await postJson<Resolution>('/api/resolve', body);
  return answer.ok ? { ok: true, status: answer.status, value: answer.value.refs } : answer;
}

/** The highlight entry the address names, why it draws nothing, or null while the site holds no conversation. */
export type HighlightEntry = { ok: true; command: PanelCommandEntry } | { ok: false; why: string };

export async function readHighlightEntry(site: string, seq: number): Promise<HighlightEntry> {
  const missing: HighlightEntry = { ok: false, why: "These highlights are not in this site's conversation, so none are drawn." };
  const listed = await getJson<ConversationList>(`/api/conversations?site=${encodeURIComponent(site)}`);
  if (!listed.ok) return { ok: false, why: `The highlights could not be read: ${listed.reason}` };
  const holder = listed.value.items.find((item) => item.first_seq <= seq && seq <= item.last_seq);
  if (holder === undefined) return missing;
  const page = await getJson<EntriesPage>(`/api/conversations/${holder.id}/entries?after=${seq - 1}&limit=1`);
  if (!page.ok) return { ok: false, why: `The highlights could not be read: ${page.reason}` };
  const entry = page.value.entries.find((held) => held.seq === seq);
  if (entry === undefined || entry.kind !== 'panel.command') return missing;
  const command = entry.body as unknown as PanelCommandEntry;
  if (command.panel !== 'map' || (command.highlight ?? []).length === 0) return missing;
  return { ok: true, command };
}
