import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';
import { ScenarioConfigSchema } from './scenario-config.schema.js';

const PatchScenarioSchema = z.object({
  name: z.string().trim().min(1).max(100).optional(),
  config: ScenarioConfigSchema.optional().meta({
    description: 'Full scenario config; replaces the stored config',
  }),
});

export class PatchScenarioDto extends createZodDto(PatchScenarioSchema) {}
