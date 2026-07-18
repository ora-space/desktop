import assert from "node:assert/strict";
import test from "node:test";

import { parseStrictJson } from "../../src/json/strict.js";

test("strict JSON rejects duplicate keys and trailing content", () => {
  assert.throws(() => parseStrictJson('{"outer":{"value":1,"value":2}}'), /duplicate/u);
  assert.throws(() => parseStrictJson('{"ok":true} false'), /trailing/u);
  assert.deepEqual(parseStrictJson('{"items":[1,true,null,"text"]}'), {
    items: [1, true, null, "text"],
  });
});

test("strict JSON enforces its recursive depth limit", () => {
  assert.throws(() => parseStrictJson('{"a":{"b":1}}', 2), /depth/u);
  assert.deepEqual(parseStrictJson('{"a":1}', 2), { a: 1 });
});
