import { decodeJsonBytes } from "../cooperativeJson";

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test.each([
  '{"__proto__":{"polluted":true},"a":1,"a":2,"constructor":null}',
  '[null,true,false,-0,1e400,{"text":"escape \\\" \\u4e2d"},[]]',
  '"' + "中🙂".repeat(100000) + '"',
  JSON.stringify(Array.from({ length: 10000 }, (_, i) => ({ id: i, values: [i, `${i}:\\\"`, null] }))),
])("large parsing matches JSON.parse without changing own properties or Unicode", async source => {
  const text = source.padEnd(300000, " ");
  const pending = decodeJsonBytes(new TextEncoder().encode(text));
  const other = jest.fn();
  setTimeout(other, 0);
  await jest.runAllTimersAsync();
  expect(await pending).toEqual(JSON.parse(text));
  expect(other).toHaveBeenCalled();
  expect(({} as Record<string, unknown>).polluted).toBeUndefined();
});

test.each(['[1,]', '{"a":1,}', '[01]', '[1 2]', '{"a" 2}', '{', 'true false', '"bad\\x"'])
("invalid large JSON is rejected: %s", async source => {
  const assertion = expect(decodeJsonBytes(new TextEncoder().encode(source.padEnd(300000, " ")))).rejects.toBeInstanceOf(SyntaxError);
  await jest.runAllTimersAsync();
  await assertion;
});

test("an obsolete read stops during cooperative decoding", async () => {
  let current = true;
  const pending = decodeJsonBytes(new TextEncoder().encode(JSON.stringify("x".repeat(1000000))), () => current);
  const assertion = expect(pending).rejects.toThrow("stale_json_decode");
  current = false;
  await jest.runAllTimersAsync();
  await assertion;
});
