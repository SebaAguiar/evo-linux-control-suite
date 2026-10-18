/**
 * commitlint — exige Conventional Commits.
 * Formato: `type(scope): subject` (ej. `feat(desktop): nueva consola horizontal`).
 * Tipos: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert.
 * Docs: https://commitlint.js.org
 */
export default {
  extends: ['@commitlint/config-conventional'],
};
