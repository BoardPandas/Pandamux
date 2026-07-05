import { describe, it, expect } from 'vitest';
import { applyPandaMUXHooks } from '../../src/main/claude-context';

const HOOK = '/res/cli/pandamux-hook.js';

const pandamuxCmds = (entries: any[]): string[] =>
  entries.flatMap((e) => (e.hooks || []).map((h: any) => h.command as string));

describe('applyPandaMUXHooks (issue #53)', () => {
  it('installs PostToolUse, Notification and Stop pandamux hooks', () => {
    const out = applyPandaMUXHooks({}, HOOK);

    // PostToolUse: one entry per tracked tool.
    const postCmds = pandamuxCmds(out.hooks.PostToolUse);
    expect(postCmds.some((c) => c.includes('pandamux-hook.js') && c.includes('Bash'))).toBe(true);
    expect(postCmds.some((c) => c.includes('Edit'))).toBe(true);

    // Notification + Stop: pass an --event flag.
    expect(pandamuxCmds(out.hooks.Notification)).toEqual([
      `node "${HOOK}" --event Notification 2>/dev/null || true`,
    ]);
    expect(pandamuxCmds(out.hooks.Stop)).toEqual([
      `node "${HOOK}" --event Stop 2>/dev/null || true`,
    ]);
  });

  it('preserves existing user hooks in every array', () => {
    const userPost = { matcher: 'Bash', hooks: [{ type: 'command', command: 'my-own-script.sh' }] };
    const userStop = { hooks: [{ type: 'command', command: 'notify-send done' }] };
    const out = applyPandaMUXHooks(
      { hooks: { PostToolUse: [userPost], Stop: [userStop] } },
      HOOK,
    );

    expect(pandamuxCmds(out.hooks.PostToolUse)).toContain('my-own-script.sh');
    expect(pandamuxCmds(out.hooks.Stop)).toContain('notify-send done');
    // ...and the pandamux entries are still added alongside them.
    expect(pandamuxCmds(out.hooks.Stop).some((c) => c.includes('--event Stop'))).toBe(true);
  });

  it('is idempotent — re-running replaces pandamux entries, never duplicates them', () => {
    const once = applyPandaMUXHooks({}, HOOK);
    const twice = applyPandaMUXHooks(once, HOOK);

    expect(twice.hooks.Notification).toHaveLength(1);
    expect(twice.hooks.Stop).toHaveLength(1);
    // Same number of PostToolUse entries on the second pass (no accumulation).
    expect(twice.hooks.PostToolUse).toHaveLength(once.hooks.PostToolUse.length);
  });

  it('does not mutate the input settings object', () => {
    const input: any = { hooks: { Stop: [{ hooks: [{ type: 'command', command: 'user' }] }] } };
    const snapshot = JSON.stringify(input);
    applyPandaMUXHooks(input, HOOK);
    expect(JSON.stringify(input)).toBe(snapshot);
  });
});
