import { useStore } from '../store';
import { Language, translate } from './core';

// Re-export the pure core so components can `import { useT, LANGUAGES, ... }
// from '../i18n'` in one place. The store-free pieces live in ./core to avoid a
// circular import with the settings slice (which imports ./core directly).
export * from './core';

/** React hook: returns a `t(key, fallback?)` bound to the current language. */
export function useT(): (key: string, fallback?: string) => string {
  const lang = useStore((s) => s.language);
  return (key: string, fallback?: string) => translate(lang as Language, key, fallback);
}
