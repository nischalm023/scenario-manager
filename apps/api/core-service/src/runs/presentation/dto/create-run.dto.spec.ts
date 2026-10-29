import { describe, expect, it } from 'vitest';
import { runEngineValues } from '../../../db/schema/enums.js';
import { CreateRunDto } from './create-run.dto.js';

const valid = {
  scenarioId: '3f2b8c1e-9d4a-4b6e-8a1f-2c7d5e9b0a13',
  engine: runEngineValues[0],
  triggeredBy: 'u1',
  correlationId: 'corr-1',
};

const parse = (input: unknown) => CreateRunDto.schema.safeParse(input);
const paths = (input: unknown) => parse(input).error?.issues.map((i) => i.path.join('.'));

describe('CreateRunDto', () => {
  it('accepts a valid body', () => {
    expect(parse(valid).success).toBe(true);
  });

  it.each(Object.keys(valid))('rejects a missing %s', (field) => {
    const { [field]: _omitted, ...body } = valid as Record<string, string>;
    expect(paths(body)).toEqual([field]);
  });

  it.each(['triggeredBy', 'correlationId'])('rejects an empty %s', (field) => {
    expect(paths({ ...valid, [field]: '' })).toEqual([field]);
  });

  it.each(['scenarioId', 'engine'])('rejects an empty %s', (field) => {
    expect(paths({ ...valid, [field]: '' })).toEqual([field]);
  });

  it('rejects a non-uuid scenarioId', () => {
    expect(paths({ ...valid, scenarioId: 's1' })).toEqual(['scenarioId']);
  });

  it('rejects an unknown engine', () => {
    expect(paths({ ...valid, engine: 'not-an-engine' })).toEqual(['engine']);
  });

  it.each(['scenarioId', 'engine', 'triggeredBy', 'correlationId'])(
    'rejects a non-string %s',
    (field) => {
      expect(paths({ ...valid, [field]: 123 })).toEqual([field]);
    },
  );

  it.each([null, 'text', 7, []])('rejects a non-object body (%j)', (body) => {
    expect(parse(body).success).toBe(false);
  });
});
