import { DomainError } from '../../common/domain/domain-error.js';

export { ProjectNotFoundError } from '../../projects/domain/errors.js';

export class MapNotFoundError extends DomainError {
  constructor(projectId: string, mapId: string) {
    super(`Map ${mapId} not found in project ${projectId}`, 'not_found');
  }
}

export class NotPermittedError extends DomainError {
  constructor(projectId: string) {
    super(`Not assigned to project ${projectId}`, 'forbidden');
  }
}
