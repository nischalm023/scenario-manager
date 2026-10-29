import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';
import { runEngineValues } from '../../../db/schema/enums.js';

const CreateRunSchema = z.object({
  scenarioId: z.uuid().meta({ description: 'Scenario to execute' }),
  engine: z.enum(runEngineValues),
  triggeredBy: z.string().min(1).meta({ description: 'Actor that triggered the run' }),
  correlationId: z.string().min(1).meta({ description: 'Idempotency key sent to the run engine' }),
});

export class CreateRunDto extends createZodDto(CreateRunSchema) {}
