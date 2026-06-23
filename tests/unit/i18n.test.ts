import { describe, it, expect } from 'vitest';
import { translate, detectDefaultLanguage, LANGUAGES, SUPPORTED_LANGUAGES } from '../../src/renderer/i18n';

// The i18n layer (issue #56) backs the Settings language switcher. These cover
// the fallback chain (active language → English → key) and locale detection so a
// partial translation never renders blank.

describe('i18n: translate (issue #56)', () => {
  it('returns the translation for the active language', () => {
    expect(translate('fr', 'settings.title')).toBe('Paramètres');
    expect(translate('zh', 'settings.title')).toBe('设置');
  });

  it('falls back to English for an untranslated key', () => {
    // A key only present in English resolves to English in every language.
    expect(translate('fr', 'palette.category.actions')).toBe('Actions');
  });

  it('falls back to the provided fallback, then the key itself', () => {
    expect(translate('en', 'nonexistent.key', 'My Fallback')).toBe('My Fallback');
    expect(translate('en', 'nonexistent.key')).toBe('nonexistent.key');
  });

  it('exposes the three shipped languages', () => {
    expect(SUPPORTED_LANGUAGES).toEqual(['en', 'fr', 'zh']);
    expect(LANGUAGES.map((l) => l.label)).toEqual(['English', 'Français', '中文']);
  });
});

describe('i18n: detectDefaultLanguage (issue #56)', () => {
  it('returns a supported language', () => {
    expect(SUPPORTED_LANGUAGES).toContain(detectDefaultLanguage());
  });
});
