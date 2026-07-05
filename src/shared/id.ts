/**
 * UUID v4 generator backed by the platform's Web Crypto implementation.
 *
 * Replaces the `uuid` npm package, which became ESM-only in v12 and would
 * otherwise break the CommonJS main-process build (`require('uuid')` has no
 * entry point). `crypto.randomUUID()` is a global in both environments we run
 * in: Node's webcrypto (main process, Node 20+) and the browser Crypto object
 * (renderer, a secure context under both http://localhost dev and file:// prod),
 * so a single helper serves both tsconfig targets. Output is an RFC 4122 v4
 * UUID string, identical in shape to the old `v4()` we replaced.
 */
export function uuid(): string {
  return crypto.randomUUID();
}
