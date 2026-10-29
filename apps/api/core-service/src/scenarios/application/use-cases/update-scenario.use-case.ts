import { Inject, Injectable } from '@nestjs/common';
import { ScenarioNotFoundError } from '../../domain/errors.js';
import { replaceConfig } from '../replace-config.js';
import {
  SCENARIO_CONFIG_READER,
  type ScenarioConfigReader,
} from '../ports/scenario-config.reader.js';
import { SCENARIO_REPOSITORY, type ScenarioRepository } from '../ports/scenario.repository.js';

export interface UpdateScenarioInput {
  id: string;
  name?: string;
  /** Full config; replaces the stored one. */
  config?: Record<string, unknown>;
  updatedBy: string;
}

@Injectable()
export class UpdateScenarioUseCase {
  constructor(
    @Inject(SCENARIO_REPOSITORY) private readonly scenarios: ScenarioRepository,
    @Inject(SCENARIO_CONFIG_READER) private readonly configs: ScenarioConfigReader,
  ) {}

  async execute(input: UpdateScenarioInput) {
    const scenario = await this.scenarios.findById(input.id);
    if (!scenario) throw new ScenarioNotFoundError(input.id);
    scenario.assertEditable();

    // The previous config is only needed to report what changed. A blob is fetched outside the row lock;
    // an inline body is read under it in the repository.
    const current = input.config ? await this.scenarios.findCurrentVersion(scenario) : null;
    const baseline =
      current && !current.config && current.configUri ? await this.configs.read(current.configUri) : undefined;

    let replaced: ReturnType<typeof replaceConfig> | undefined;
    const updated = await this.scenarios.update(input.id, {
      name: input.name,
      patchConfig: input.config
        ? (stored) => {
            replaced = replaceConfig(stored ?? baseline ?? {}, input.config!);
            return replaced.config;
          }
        : undefined,
      updatedBy: input.updatedBy,
    });
    return { scenario: updated, changes: replaced?.changes ?? [], modules: replaced?.modules ?? [] };
  }
}
