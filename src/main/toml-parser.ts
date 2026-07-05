/**
 * toml-parser.ts — Minimal TOML parser.
 *
 * Supports the subset we need for ~/.pandamux/config.toml:
 *   [table], [nested.table], [with."quoted.segment"]
 *   key = "string" | 'literal' | number | true | false | [array]
 *   # comments (ignored; preserved inside strings)
 *
 * Not supported (intentionally, to keep this hand-rolled parser small):
 *   Inline tables `{ a = 1 }`, datetime literals, multi-line strings,
 *   hex/oct/bin numbers, underscores inside numbers.
 *
 * Throws on malformed input so callers can decide whether to fall back
 * to defaults or surface an error.
 */
export type TomlValue = string | number | boolean | TomlValue[] | TomlTable;
export interface TomlTable { [key: string]: TomlValue; }

export function parseToml(input: string): TomlTable {
  const root: TomlTable = {};
  let current: TomlTable = root;

  const lines = input.split(/\r?\n/);
  let i = 0;

  while (i < lines.length) {
    let line = stripComment(lines[i]).trim();
    i++;
    if (!line) continue;

    if (line.startsWith('[')) {
      const end = line.lastIndexOf(']');
      if (end === -1) throw new Error(`Unclosed table header: ${line}`);
      const path = splitDottedKey(line.slice(1, end).trim());
      current = ensureTable(root, path);
      continue;
    }

    const eq = findTopLevelEquals(line);
    if (eq === -1) continue;

    const rawKey = line.slice(0, eq).trim();
    const key = unquoteKey(rawKey);
    let rest = line.slice(eq + 1).trim();

    // Arrays may span multiple lines; accumulate until brackets balance.
    if (rest.startsWith('[')) {
      while (bracketDepth(rest) > 0 && i < lines.length) {
        rest += ' ' + stripComment(lines[i]).trim();
        i++;
      }
    }

    current[key] = parseValue(rest);
  }

  return root;
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

function stripComment(line: string): string {
  let inStr: string | null = null;
  for (let j = 0; j < line.length; j++) {
    const c = line[j];
    if (inStr) {
      if (c === '\\' && inStr === '"') { j++; continue; }
      if (c === inStr) inStr = null;
    } else {
      if (c === '"' || c === "'") inStr = c;
      else if (c === '#') return line.slice(0, j);
    }
  }
  return line;
}

function findTopLevelEquals(line: string): number {
  let inStr: string | null = null;
  for (let j = 0; j < line.length; j++) {
    const c = line[j];
    if (inStr) {
      if (c === '\\' && inStr === '"') { j++; continue; }
      if (c === inStr) inStr = null;
    } else {
      if (c === '"' || c === "'") inStr = c;
      else if (c === '=') return j;
    }
  }
  return -1;
}

function unquoteKey(raw: string): string {
  if ((raw.startsWith('"') && raw.endsWith('"')) ||
      (raw.startsWith("'") && raw.endsWith("'"))) {
    return raw.slice(1, -1);
  }
  return raw;
}

function splitDottedKey(s: string): string[] {
  const out: string[] = [];
  let cur = '';
  let inStr: string | null = null;
  for (let j = 0; j < s.length; j++) {
    const c = s[j];
    if (inStr) {
      if (c === inStr) inStr = null;
      else cur += c;
    } else {
      if (c === '"' || c === "'") inStr = c;
      else if (c === '.') { if (cur.trim()) out.push(cur.trim()); cur = ''; }
      else cur += c;
    }
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

function ensureTable(root: TomlTable, path: string[]): TomlTable {
  let cur = root;
  for (const part of path) {
    const existing = cur[part];
    if (existing && typeof existing === 'object' && !Array.isArray(existing)) {
      cur = existing as TomlTable;
    } else {
      const fresh: TomlTable = {};
      cur[part] = fresh;
      cur = fresh;
    }
  }
  return cur;
}

function bracketDepth(s: string): number {
  let depth = 0;
  let inStr: string | null = null;
  for (let j = 0; j < s.length; j++) {
    const c = s[j];
    if (inStr) {
      if (c === '\\' && inStr === '"') { j++; continue; }
      if (c === inStr) inStr = null;
    } else {
      if (c === '"' || c === "'") inStr = c;
      else if (c === '[') depth++;
      else if (c === ']') depth--;
    }
  }
  return depth;
}

function parseValue(raw: string): TomlValue {
  const s = raw.trim();
  if (!s.length) return '';

  // Strings
  if (s.startsWith('"')) return parseDoubleQuoted(s);
  if (s.startsWith("'")) return parseSingleQuoted(s);

  // Booleans
  if (s === 'true') return true;
  if (s === 'false') return false;

  // Arrays
  if (s.startsWith('[')) {
    const close = s.lastIndexOf(']');
    if (close === -1) throw new Error(`Unclosed array: ${s}`);
    return splitArrayItems(s.slice(1, close)).map(parseValue);
  }

  // Numbers
  if (/^[-+]?\d+$/.test(s)) return parseInt(s, 10);
  if (/^[-+]?(\d+\.\d+|\.\d+|\d+\.)([eE][-+]?\d+)?$/.test(s)) return parseFloat(s);
  if (/^[-+]?\d+[eE][-+]?\d+$/.test(s)) return parseFloat(s);

  // Fallback: treat as bare string so typos degrade gracefully.
  return s;
}

function parseDoubleQuoted(s: string): string {
  let out = '';
  let j = 1;
  while (j < s.length) {
    const c = s[j];
    if (c === '\\') {
      const n = s[j + 1];
      if (n === 'n') out += '\n';
      else if (n === 't') out += '\t';
      else if (n === 'r') out += '\r';
      else if (n === '\\') out += '\\';
      else if (n === '"') out += '"';
      else if (n === "'") out += "'";
      else out += n;
      j += 2;
    } else if (c === '"') {
      return out;
    } else {
      out += c;
      j++;
    }
  }
  throw new Error(`Unterminated string: ${s}`);
}

function parseSingleQuoted(s: string): string {
  // TOML literal strings: no escapes.
  const close = s.indexOf("'", 1);
  if (close === -1) throw new Error(`Unterminated literal string: ${s}`);
  return s.slice(1, close);
}

function splitArrayItems(inner: string): string[] {
  const out: string[] = [];
  let cur = '';
  let inStr: string | null = null;
  let depth = 0;
  for (let j = 0; j < inner.length; j++) {
    const c = inner[j];
    if (inStr) {
      cur += c;
      if (c === '\\' && inStr === '"') {
        cur += inner[j + 1] ?? '';
        j++;
      } else if (c === inStr) {
        inStr = null;
      }
    } else {
      if (c === '"' || c === "'") { inStr = c; cur += c; }
      else if (c === '[') { depth++; cur += c; }
      else if (c === ']') { depth--; cur += c; }
      else if (c === ',' && depth === 0) {
        const item = cur.trim();
        if (item.length) out.push(item);
        cur = '';
      } else cur += c;
    }
  }
  const last = cur.trim();
  if (last.length) out.push(last);
  return out;
}
