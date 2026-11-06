/** Bounded JSON decoding for hostile RPC input. JSON.parse alone discards
 * duplicate object members, including conflicting receipt status/hash fields. */
export function parseStrictJson(text: string): unknown {
  if (text.length > 1_048_576) throw new Error("JSON response is too large.");
  let at = 0;
  let nodes = 0;
  const fail = (): never => { throw new Error("Malformed or ambiguous network JSON."); };
  const space = () => { while (at < text.length && /[\x20\x09\x0a\x0d]/.test(text[at])) at++; };
  const string = (): string => {
    if (text[at] !== '"') return fail();
    const start = at++;
    while (at < text.length) {
      const character = text[at++];
      if (character === '"') {
        try { return JSON.parse(text.slice(start, at)) as string; } catch { return fail(); }
      }
      if (character === "\\") { if (at === text.length) return fail(); at++; }
      else if (character.charCodeAt(0) < 32) return fail();
    }
    return fail();
  };
  const value = (depth: number): unknown => {
    if (depth > 32 || ++nodes > 50_000) return fail();
    space();
    const c = text[at];
    if (c === '"') return string();
    if (c === "{") {
      at++; space();
      const result: Record<string, unknown> = Object.create(null);
      const names = new Set<string>();
      if (text[at] === "}") { at++; return result; }
      for (;;) {
        space();
        const name = string();
        if (names.has(name)) return fail();
        names.add(name); space();
        if (text[at++] !== ":") return fail();
        result[name] = value(depth + 1); space();
        const delimiter = text[at++];
        if (delimiter === "}") return result;
        if (delimiter !== ",") return fail();
      }
    }
    if (c === "[") {
      at++; space();
      const result: unknown[] = [];
      if (text[at] === "]") { at++; return result; }
      for (;;) {
        result.push(value(depth + 1)); space();
        const delimiter = text[at++];
        if (delimiter === "]") return result;
        if (delimiter !== ",") return fail();
      }
    }
    for (const [literal, result] of [["true", true], ["false", false], ["null", null]] as const) {
      if (text.startsWith(literal, at)) { at += literal.length; return result; }
    }
    const number = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(text.slice(at));
    if (!number) return fail();
    at += number[0].length;
    const parsed = Number(number[0]);
    if (!Number.isFinite(parsed) || (Number.isInteger(parsed) && !Number.isSafeInteger(parsed))) return fail();
    return parsed;
  };
  const result = value(0);
  space();
  if (at !== text.length) return fail();
  return result;
}
