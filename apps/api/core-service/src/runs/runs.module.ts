import { Module } from '@nestjs/common';
import { ScenariosModule } from '../scenarios/scenarios.module.js';
import { RUN_REPOSITORY } from './application/ports/run.repository.js';
import { RUN_TRIGGER } from './application/ports/run-trigger.port.js';
import { TriggerRunUseCase } from './application/use-cases/trigger-run.use-case.js';
import { InMemoryRunRepository } from './infrastructure/in-memory-run.repository.js';
import { HttpRunTriggerAdapter } from './infrastructure/http-run-trigger.adapter.js';
import { RunsController } from './presentation/runs.controller.js';

@Module({
  imports: [ScenariosModule],
  controllers: [RunsController],
  providers: [
    TriggerRunUseCase,
    { provide: RUN_REPOSITORY, useClass: InMemoryRunRepository },
    { provide: RUN_TRIGGER, useClass: HttpRunTriggerAdapter },
  ],
})
export class RunsModule {}
