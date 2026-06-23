import { useStore } from '../../store';
import { LANGUAGES, Language, useT } from '../../i18n';

// General settings — currently the UI language switcher (issue #56). The app
// previously had no way to change language from the gear page; this is the
// "language switching module" the issue asked for.
export default function GeneralSettings() {
  const language = useStore((s) => s.language);
  const setLanguage = useStore((s) => s.setLanguage);
  const t = useT();

  return (
    <div className="settings-section">
      <h3 className="settings-section-title">{t('settings.general.languageSection')}</h3>

      <div className="settings-row">
        <label className="settings-label">{t('settings.general.language')}</label>
        <select
          className="settings-select"
          value={language}
          onChange={(e) => setLanguage(e.target.value as Language)}
        >
          {LANGUAGES.map((lang) => (
            <option key={lang.code} value={lang.code}>
              {lang.label}
            </option>
          ))}
        </select>
      </div>

      <p className="settings-hint">{t('settings.general.languageHint')}</p>
    </div>
  );
}
