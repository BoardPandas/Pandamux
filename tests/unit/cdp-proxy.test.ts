import { describe, it, expect, vi } from 'vitest';

// cdp-proxy.ts imports `electron` at module scope; stub it so the pure
// host-check function can be tested without an Electron runtime.
vi.mock('electron', () => ({
  webContents: { fromId: () => undefined },
}));

import { isAllowedCdpHost, isAllowedCdpOrigin } from '../../src/main/cdp-proxy';

describe('isAllowedCdpHost (DNS-rebinding guard)', () => {
  it('allows loopback literals with the proxy port', () => {
    expect(isAllowedCdpHost('localhost:9222')).toBe(true);
    expect(isAllowedCdpHost('127.0.0.1:9222')).toBe(true);
    expect(isAllowedCdpHost('localhost')).toBe(true);
    expect(isAllowedCdpHost('127.0.0.1')).toBe(true);
  });

  it('allows IPv6 loopback', () => {
    expect(isAllowedCdpHost('[::1]:9222')).toBe(true);
    expect(isAllowedCdpHost('[::1]')).toBe(true);
    expect(isAllowedCdpHost('::1')).toBe(true);
  });

  it('allows requests with no Host header (native ws clients)', () => {
    expect(isAllowedCdpHost(undefined)).toBe(true);
  });

  it('rejects attacker-controlled hostnames that rebind to loopback', () => {
    expect(isAllowedCdpHost('evil.com')).toBe(false);
    expect(isAllowedCdpHost('evil.com:9222')).toBe(false);
    expect(isAllowedCdpHost('attacker.localhost.evil.com:9222')).toBe(false);
    expect(isAllowedCdpHost('localhost.evil.com')).toBe(false);
  });

  it('rejects non-loopback IPs', () => {
    expect(isAllowedCdpHost('0.0.0.0:9222')).toBe(false);
    expect(isAllowedCdpHost('192.168.1.10:9222')).toBe(false);
    expect(isAllowedCdpHost('10.0.0.1')).toBe(false);
  });

  it('is case-insensitive', () => {
    expect(isAllowedCdpHost('LOCALHOST:9222')).toBe(true);
  });
});

describe('isAllowedCdpOrigin (browser-origin guard)', () => {
  it('allows requests with no Origin header (native CDP clients)', () => {
    expect(isAllowedCdpOrigin(undefined)).toBe(true);
    expect(isAllowedCdpOrigin('')).toBe(true);
  });

  it('allows the DevTools front-end scheme', () => {
    expect(isAllowedCdpOrigin('devtools://devtools')).toBe(true);
  });

  it('rejects web origins even when they point at loopback', () => {
    expect(isAllowedCdpOrigin('http://127.0.0.1:9222')).toBe(false);
    expect(isAllowedCdpOrigin('http://localhost:3000')).toBe(false);
    expect(isAllowedCdpOrigin('https://evil.com')).toBe(false);
    expect(isAllowedCdpOrigin('null')).toBe(false);
    expect(isAllowedCdpOrigin('file://')).toBe(false);
  });
});
