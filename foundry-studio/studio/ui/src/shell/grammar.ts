// Spec 03 section 3.1, the reference grammar, read in the browser as far as routes and `data-ref` need it: scope
// paths, pins, locations, `kin` pins, object keys, entity references, keys and references. The Rust parser in
// `studio/protocol` is the authority; this reading passes the same golden vectors
// (`studio/protocol/tests/grammar-vectors.json`), so an address the API would refuse is refused here first.
// Each parser returns the canonical text (rule 7) or throws a `GrammarError` naming the string and why.

import type { ObjectKind } from '../protocol/grammar.ts';

const MAX_SLUGS = 8;
const MAX_SLUG_CHARS = 64;
const TAG_PREFIX = 'tag:';
const SCOPE_PREFIX = 'scope:';
const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const OBJECT_KINDS: readonly ObjectKind[] = [
  'conversation', 'scenario', 'experiment', 'run', 'submission', 'draft', 'template-draft',
];

export class GrammarError extends Error {
  readonly form: string;
  readonly input: string;
  readonly why: string;

  constructor(form: string, input: string, why: string) {
    super(`\`${input}\` is not ${form}: ${why}`);
    this.form = form;
    this.input = input;
    this.why = why;
  }
}

function refuse(form: string, input: string, why: string): never {
  throw new GrammarError(form, input, why);
}

/** Read `inner` as part of `text`, and say any failure of the part as a failure of the whole. */
function within<T>(form: string, text: string, inner: () => T): T {
  try {
    return inner();
  } catch (fault) {
    if (fault instanceof GrammarError) refuse(form, text, fault.why);
    throw fault;
  }
}

function slugFault(slug: string): string | null {
  if (slug === '') return 'a slug is never empty';
  const odd = [...slug].find((c) => !/[a-z0-9-]/.test(c));
  if (odd !== undefined) return `the slug \`${slug}\` holds \`${odd}\`, outside a-z, 0-9 and -`;
  if (slug.length > MAX_SLUG_CHARS) return `the slug \`${slug}\` is longer than ${MAX_SLUG_CHARS} characters`;
  if (slug.startsWith('-') || slug.endsWith('-')) return `the slug \`${slug}\` must start and end with a letter or digit`;
  return null;
}

const allDigits = (text: string): boolean => /^[0-9]+$/.test(text);

export function parseScopePath(text: string): string {
  const slugs = text.split('/');
  if (slugs.length > MAX_SLUGS) {
    refuse('a scope path', text, `it has ${slugs.length} slugs, and a scope path has at most ${MAX_SLUGS}`);
  }
  for (const slug of slugs) {
    const why = slugFault(slug);
    if (why !== null) refuse('a scope path', text, why);
  }
  return text;
}

/** A site's root scope, `customer/{c}/site/{s}`, or null when `path` is not one. */
export function siteRootOf(path: string): string | null {
  const slugs = path.split('/');
  return slugs.length >= 4 && slugs[0] === 'customer' && slugs[2] === 'site' ? slugs.slice(0, 4).join('/') : null;
}

/** A site template's root scope, `template/site/{t}`, or null when `path` is not under one. */
export function templateRootOf(path: string): string | null {
  const slugs = path.split('/');
  return slugs.length >= 3 && slugs[0] === 'template' && slugs[1] === 'site' ? slugs.slice(0, 3).join('/') : null;
}

export function parseSitePath(text: string): string {
  within('a site path', text, () => parseScopePath(text));
  if (siteRootOf(text) !== text) refuse('a site path', text, "a site's root scope is customer/{customer}/site/{site}");
  return text;
}

export function parseSeq(text: string): number {
  if (!allDigits(text)) refuse('a changeset', text, 'a changeset is written in digits');
  if (text.length > 1 && text.startsWith('0')) refuse('a changeset', text, 'a changeset is written without leading zeros');
  const value = Number(text);
  if (!Number.isSafeInteger(value)) refuse('a changeset', text, 'it is larger than any changeset');
  return value;
}

export function parseBranch(text: string): string {
  const why = slugFault(text);
  if (why !== null) refuse('a branch name', text, why);
  if (allDigits(text)) refuse('a branch name', text, 'an all-digit name reads as a changeset');
  if (text === 'head') refuse('a branch name', text, '`head` is a pin of entity references only, never a branch');
  return text;
}

export type Pin =
  | { kind: 'changeset'; seq: number }
  | { kind: 'tag'; name: string }
  | { kind: 'branch'; name: string };

export function parsePin(text: string): Pin {
  return within('a pin', text, (): Pin => {
    if (text.startsWith(TAG_PREFIX)) {
      const name = text.slice(TAG_PREFIX.length);
      const why = slugFault(name);
      if (why !== null) refuse('a tag name', name, why);
      return { kind: 'tag', name };
    }
    if (allDigits(text)) return { kind: 'changeset', seq: parseSeq(text) };
    return { kind: 'branch', name: parseBranch(text) };
  });
}

export function printPin(pin: Pin): string {
  switch (pin.kind) {
    case 'changeset': return String(pin.seq);
    case 'tag': return `${TAG_PREFIX}${pin.name}`;
    case 'branch': return pin.name;
  }
}

export interface Location {
  scope: string;
  pin: Pin;
}

/** Rule 2: a location never says `@head`, and one without a pin follows `main`. */
export function parseLocation(text: string): Location {
  const at = text.indexOf('@');
  if (at < 0) return { scope: within('a location', text, () => parseScopePath(text)), pin: { kind: 'branch', name: 'main' } };
  const [path, pin] = [text.slice(0, at), text.slice(at + 1)];
  if (pin.includes('@')) refuse('a location', text, 'a location holds one `@`');
  if (pin === 'head') refuse('a location', text, 'a location never says `@head`: it has no referrer whose branch to follow');
  const parsedPin = within('a location', text, () => parsePin(pin));
  return { scope: within('a location', text, () => parseScopePath(path)), pin: parsedPin };
}

export const printLocation = (location: Location): string => `${location.scope}@${printPin(location.pin)}`;

export interface KinPin {
  scope: string;
  seq: number;
}

export function parseKinPins(text: string): KinPin[] {
  const pins = text.split(',').map((one): KinPin => {
    const at = one.indexOf('@');
    if (at < 0) refuse('kin pins', text, 'a kin pin names its changeset, as `flow@2` does');
    const [path, changeset] = [one.slice(0, at), one.slice(at + 1)];
    if (changeset.includes('@')) refuse('kin pins', text, 'a kin pin holds one `@`');
    if (!allDigits(changeset)) refuse('kin pins', text, 'a kin pin holds its scope at a changeset, never at a branch or a tag');
    return within('kin pins', text, () => ({ scope: parseScopePath(path), seq: parseSeq(changeset) }));
  });
  pins.forEach((pin, n) => {
    if (pins.slice(0, n).some((earlier) => earlier.scope === pin.scope)) refuse('kin pins', text, `they name \`${pin.scope}\` twice`);
  });
  return pins;
}

export const printKinPins = (pins: KinPin[]): string => pins.map((pin) => `${pin.scope}@${pin.seq}`).join(',');

/** Spec 03 section 3.2, `kin`: only beside a changeset location, each a scope under the site, never the shown one. */
export function checkKinBeside(pins: KinPin[], shown: Location, site: string): void {
  const text = printKinPins(pins);
  if (shown.pin.kind !== 'changeset') {
    refuse('kin pins', text, `kin pins stand only beside a changeset location, never beside \`${printLocation(shown)}\``);
  }
  if (!(shown.scope === site || shown.scope.startsWith(`${site}/`))) {
    refuse('kin pins', text, `the shown scope \`${shown.scope}\` is not under the site \`${site}\``);
  }
  for (const pin of pins) {
    const scope = `${site}/${pin.scope}`;
    within('kin pins', text, () => parseScopePath(scope));
    if (scope === shown.scope) refuse('kin pins', text, `the kin pin \`${pin.scope}@${pin.seq}\` names the shown scope \`${shown.scope}\``);
  }
}

export function parseUlid(text: string): string {
  const odd = [...text].find((c) => !CROCKFORD.includes(c));
  if (odd !== undefined) refuse('a ULID', text, `\`${odd}\` is not in Crockford's base32`);
  if (text.length !== 26) refuse('a ULID', text, `a ULID is 26 characters, and this is ${text.length}`);
  return text;
}

export interface ObjectKey {
  kind: ObjectKind;
  id: string;
}

export function parseObjectKey(text: string): ObjectKey {
  const colon = text.indexOf(':');
  if (colon < 0) refuse('an object key', text, 'an object key is a kind and a ULID, as `run:01KN0TJZANP5N0PGJD8F7A3FDB` is');
  const word = text.slice(0, colon);
  const kind = OBJECT_KINDS.find((known) => known === word);
  if (kind === undefined) refuse('an object key', text, `\`${word}\` is not a kind of object`);
  return { kind, id: within('an object key', text, () => parseUlid(text.slice(colon + 1))) };
}

function parseFieldKey(text: string): string {
  const hash = text.indexOf('#');
  const [object, pointer] = [text.slice(0, hash), text.slice(hash + 1)];
  if (pointer !== '' && !pointer.startsWith('/')) refuse('a field key', text, 'a JSON pointer is empty or starts with `/`');
  if (pointer.split('~').slice(1).some((after) => !(after.startsWith('0') || after.startsWith('1')))) {
    refuse('a field key', text, 'in a JSON pointer, `~` is followed by `0` or `1`');
  }
  within('a field key', text, () => parseObjectKey(object));
  return text;
}

const opensWithUuid = (text: string): boolean => /^[0-9a-fA-F]{8}-/.test(text);

function parseDeclaredId(text: string): string {
  if (!/^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/.test(text)) {
    refuse('a UUID', text, 'a UUID is 36 characters, hex digits grouped 8-4-4-4-12 by hyphens');
  }
  if (text !== text.toLowerCase()) refuse('a UUID', text, 'a UUID is written in lower case');
  return text;
}

export function parseEntityId(text: string): string {
  within('an entity id', text, () => parseDeclaredId(text.slice(0, 36)));
  const chain = text.slice(36);
  if (chain !== '' && !chain.startsWith('/')) refuse('an entity id', text, "a derived entity's key chain follows its UUID after a `/`");
  for (const segment of chain.split('/').slice(1)) {
    if (segment === '') refuse('an entity id', text, "a key chain's segment is never empty");
    if (segment.includes('@')) refuse('an entity id', text, 'an entity id holds no `@`: a pin belongs to a reference, never to a key');
    if (/\p{Cc}/u.test(segment)) refuse('an entity id', text, "a key chain's segment holds no control character");
  }
  return text;
}

export type RefPin = { kind: 'head' } | Pin;

export interface EntityRef {
  id: string;
  pin: RefPin | null;
}

export function parseEntityRef(text: string): EntityRef {
  const at = text.indexOf('@');
  if (at < 0) return { id: within('an entity reference', text, () => parseEntityId(text)), pin: null };
  const pin = text.slice(at + 1);
  if (pin.includes('@')) refuse('an entity reference', text, 'a reference holds one `@`');
  const id = within('an entity reference', text, () => parseEntityId(text.slice(0, at)));
  return { id, pin: pin === 'head' ? { kind: 'head' } : within('an entity reference', text, () => parsePin(pin)) };
}

export const printEntityRef = (ref: EntityRef): string =>
  ref.pin === null ? ref.id : `${ref.id}@${ref.pin.kind === 'head' ? 'head' : printPin(ref.pin)}`;

/** What a page element's `data-ref` holds; rule 6: a key never carries a pin. The canonical text. */
export function parseKey(text: string): string {
  if (text.startsWith(SCOPE_PREFIX)) {
    const path = text.slice(SCOPE_PREFIX.length);
    if (path.includes('@')) refuse('a key', text, 'a key never carries a pin');
    return `${SCOPE_PREFIX}${within('a key', text, () => parseScopePath(path))}`;
  }
  if (opensWithUuid(text)) return within('a key', text, () => parseEntityId(text));
  if (text.includes('#')) return within('a key', text, () => parseFieldKey(text));
  const object = within('a key', text, () => parseObjectKey(text));
  return `${object.kind}:${object.id}`;
}

export type Ref =
  | { kind: 'entity'; ref: EntityRef }
  | { kind: 'scope'; location: Location }
  | { kind: 'object'; object: ObjectKey };

export function parseRef(text: string): Ref {
  if (text.startsWith(SCOPE_PREFIX)) {
    return { kind: 'scope', location: within('a reference', text, () => parseLocation(text.slice(SCOPE_PREFIX.length))) };
  }
  if (opensWithUuid(text)) return { kind: 'entity', ref: within('a reference', text, () => parseEntityRef(text)) };
  if (text.includes('#')) refuse('a reference', text, 'a field key is a key only, never a reference');
  return { kind: 'object', object: within('a reference', text, () => parseObjectKey(text)) };
}

export function printRef(ref: Ref): string {
  switch (ref.kind) {
    case 'entity': return printEntityRef(ref.ref);
    case 'scope': return `${SCOPE_PREFIX}${printLocation(ref.location)}`;
    case 'object': return `${ref.object.kind}:${ref.object.id}`;
  }
}

/** The key a page element carries for what `ref` names: its pin dropped (rule 6). */
export function keyOf(ref: Ref): string {
  switch (ref.kind) {
    case 'entity': return ref.ref.id;
    case 'scope': return `${SCOPE_PREFIX}${ref.location.scope}`;
    case 'object': return `${ref.object.kind}:${ref.object.id}`;
  }
}

export type Kinship = 'kin' | 'lateral';

/** Rules 4 and 5: the reference as a command keeps it, or the problem type of the rule it breaks. */
export function underPinRule(ref: EntityRef, kinship: Kinship): EntityRef | 'ref-pin-conflicts' | 'ref-needs-version' {
  if (kinship === 'kin') {
    if (ref.pin === null) return ref;
    return ref.pin.kind === 'head' ? { id: ref.id, pin: null } : 'ref-pin-conflicts';
  }
  return ref.pin !== null && (ref.pin.kind === 'changeset' || ref.pin.kind === 'tag') ? ref : 'ref-needs-version';
}
