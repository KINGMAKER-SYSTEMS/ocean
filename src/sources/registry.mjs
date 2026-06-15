import { createGitHubSourceRunner } from './github.mjs';
import { createTelegramSourceRunner } from './telegram.mjs';

const RUNNERS = new Map([
  ['github', createGitHubSourceRunner],
  ['telegram', createTelegramSourceRunner],
]);

export function listRunnerAdapters() {
  return [...RUNNERS.keys()].sort();
}

export function hasRunner(adapterId) {
  return RUNNERS.has(adapterId);
}

export function getRunner(adapterId) {
  const factory = RUNNERS.get(adapterId);
  if (!factory) {
    const error = new Error(`no runner implemented for ${adapterId}`);
    error.code = 'NO_SOURCE_RUNNER';
    throw error;
  }
  return factory();
}
