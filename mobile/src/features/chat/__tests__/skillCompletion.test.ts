import { completeSkill, filterSkills, insertSkillSlash, skillQuery } from "../skillCompletion";

const caret = (start: number) => ({ start, end: start });

test.each(["/", "/research", "请使用 /研究", "hello\n/future-web"])("recognizes standalone skill token %s", text => {
  expect(skillQuery(text, caret(text.length))?.query).toBe(text.slice(text.lastIndexOf("/") + 1));
});

test.each(["https://", "https://example.com/foo", "C:/skills", "a/b", "/usr/local/bin", "\\server\\folder", "//server", "/foo\\bar", "/web done"])("does not complete URL/path/finished token %s", text => {
  expect(skillQuery(text, caret(text.length))).toBeNull();
});

test("validates the entire token even when the caret is inside a path", () => {
  expect(skillQuery("/usr/local", caret(4))).toBeNull();
  expect(skillQuery("/web", { start: 1, end: 4 })).toBeNull();
  expect(skillQuery("/web", caret(99))).toBeNull();
  expect(skillQuery("ask /research later", caret(7))).toEqual({ start: 4, end: 13, query: "re" });
});

test("button inserts at caret with boundaries, preserving all draft text", () => {
  expect(insertSkillSlash("", caret(0))).toEqual({ text: "/", selection: caret(1) });
  expect(insertSkillSlash("你好世界", caret(2))).toEqual({ text: "你好 / 世界", selection: caret(4) });
  expect(insertSkillSlash("hello world", caret(6))).toEqual({ text: "hello / world", selection: caret(7) });
  expect(insertSkillSlash("hello", caret(99))).toEqual({ text: "hello /", selection: caret(7) });
  expect(insertSkillSlash("hello world", { start: 6, end: 11 })).toEqual({ text: "hello / world", selection: caret(7) });
  expect(insertSkillSlash("/re", caret(3))).toEqual({ text: "/re", selection: caret(3) });
});

test("completion replaces the whole token only and leaves a space after the command", () => {
  const text = "ask /research later";
  expect(completeSkill(text, skillQuery(text, caret(7))!, "future-web"))
    .toEqual({ text: "ask /future-web later", selection: caret(16) });
  expect(completeSkill("/", skillQuery("/", caret(1))!, "web"))
    .toEqual({ text: "/web ", selection: caret(5) });
  expect(completeSkill("/re\nnext", skillQuery("/re\nnext", caret(3))!, "web"))
    .toEqual({ text: "/web \nnext", selection: caret(5) });
});

test("search matches localized names/descriptions and excludes malformed commands", () => {
  const skills = [
    { name: "future-web", description: "Search pages", nameZh: "网页搜索", descriptionZh: "读取页面" },
    { name: "future-image", description: "Generate images" },
    { name: "bad/skill", description: "Search" },
  ];
  for (const query of ["WEB", "search", "网页", "读取"]) {
    expect(filterSkills(skills, query)).toEqual([skills[0]]);
  }
  expect(filterSkills(skills, "")).toHaveLength(2);
  expect(filterSkills(skills, "不存在")).toEqual([]);
});
