import { describe, it, expect, vi } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: { getAllWindows: () => [] },
  dialog: { showMessageBox: vi.fn() },
  app: { getVersion: () => '0.0.0' },
  net: { request: vi.fn() },
}));

vi.mock('electron-updater', () => ({
  autoUpdater: {
    autoDownload: true,
    autoInstallOnAppQuit: true,
    on: vi.fn(),
    checkForUpdates: vi.fn().mockResolvedValue(undefined),
    downloadUpdate: vi.fn(),
    quitAndInstall: vi.fn(),
  },
}));

import { isMissingChannelFileError, isUpdaterDisabled } from '../../src/main/updater';

describe('isMissingChannelFileError', () => {
  it('matches by error code', () => {
    const err = Object.assign(new Error('some wrapper text'), {
      code: 'ERR_UPDATER_CHANNEL_FILE_NOT_FOUND',
    });
    expect(isMissingChannelFileError(err)).toBe(true);
  });

  it('matches the electron-updater 404 message', () => {
    const err = new Error(
      'Cannot find latest.yml in the latest release artifacts ' +
      '(https://github.com/BoardPandas/Pandamux/releases/download/v0.15.0/latest.yml): HttpError: 404'
    );
    expect(isMissingChannelFileError(err)).toBe(true);
  });

  it('matches when the code only appears in the message', () => {
    expect(isMissingChannelFileError(new Error("code: 'ERR_UPDATER_CHANNEL_FILE_NOT_FOUND'"))).toBe(true);
  });

  it('does not match unrelated errors', () => {
    expect(isMissingChannelFileError(new Error('net::ERR_INTERNET_DISCONNECTED'))).toBe(false);
    expect(isMissingChannelFileError(null)).toBe(false);
    expect(isMissingChannelFileError(undefined)).toBe(false);
    expect(isMissingChannelFileError('plain string error')).toBe(false);
  });
});

describe('isUpdaterDisabled', () => {
  it('is disabled only when PANDAMUX_DISABLE_UPDATER is exactly "1"', () => {
    expect(isUpdaterDisabled({ PANDAMUX_DISABLE_UPDATER: '1' } as NodeJS.ProcessEnv)).toBe(true);
    expect(isUpdaterDisabled({ PANDAMUX_DISABLE_UPDATER: 'true' } as NodeJS.ProcessEnv)).toBe(false);
    expect(isUpdaterDisabled({ PANDAMUX_DISABLE_UPDATER: '0' } as NodeJS.ProcessEnv)).toBe(false);
    expect(isUpdaterDisabled({} as NodeJS.ProcessEnv)).toBe(false);
  });
});
