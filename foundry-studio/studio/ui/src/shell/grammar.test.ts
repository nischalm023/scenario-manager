// Spec 03 section 8, statement 1 (the grammar), in TypeScript: every vector of the golden file the Rust parser reads
// parses and prints back canonical, each refused form is refused, rules 4 and 5 keep or refuse as the file says, and
// `kin` pins beside a location are checked as spec 03 section 3.2 says.

import { describe, expect, it } from 'vitest';
import vectors from '../../../../studio/protocol/tests/grammar-vectors.json';
import {
  GrammarError, checkKinBeside, parseBranch, parseEntityRef, parseKey, parseKinPins, parseLocation, parseObjectKey,
  parsePin, parseRef, parseScopePath, printEntityRef, printKinPins, printLocation, printPin, printRef, underPinRule,
} from './grammar.ts';

function reprint(form: string, text: string): string {
  switch (form) {
    case 'scope-path': return parseScopePath(text);
    case 'branch': return parseBranch(text);
    case 'pin': return printPin(parsePin(text));
    case 'location': return printLocation(parseLocation(text));
    case 'kin-pins': return printKinPins(parseKinPins(text));
    case 'object': { const key = parseObjectKey(text); return `${key.kind}:${key.id}`; }
    case 'entity-ref': return printEntityRef(parseEntityRef(text));
    case 'key': return parseKey(text);
    case 'ref': return printRef(parseRef(text));
    default: throw new Error(`the vector file names a form \`${form}\` this test does not know`);
  }
}

describe('spec 03 statement 1: the reference grammar on the golden vectors', () => {
  it.each(vectors.parse)('reads $form `$text` and prints it canonical', (vector) => {
    const canonical = 'canonical' in vector && typeof vector.canonical === 'string' ? vector.canonical : vector.text;
    expect(reprint(vector.form, vector.text)).toBe(canonical);
  });

  it.each(vectors.refuse)('refuses $form `$text`', (vector) => {
    expect(() => reprint(vector.form, vector.text)).toThrow(GrammarError);
  });

  it.each(vectors.pin_rule)('applies rules 4 and 5 to `$ref` ($kinship)', (vector) => {
    const outcome = underPinRule(parseEntityRef(vector.ref), vector.kinship as 'kin' | 'lateral');
    if ('refused' in vector && vector.refused) expect(outcome).toBe(vector.refused);
    else expect(typeof outcome === 'string' ? outcome : printEntityRef(outcome)).toBe((vector as { keeps: string }).keeps);
  });

  it.each(vectors.kin_beside)('checks kin `$kin` beside `$location`', (vector) => {
    const check = () => checkKinBeside(parseKinPins(vector.kin), parseLocation(vector.location), vector.site);
    if (vector.refused) expect(check).toThrow(GrammarError);
    else expect(check).not.toThrow();
  });
});
