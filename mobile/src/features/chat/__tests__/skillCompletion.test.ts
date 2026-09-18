import { completeSkill, filterActions, filterSkills, insertSkillSlash, removeSkillQuery, skillQuery } from "../skillCompletion";

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

test("context actions match their localized label and English command words", () => {
  const action = {
    id: "compact",
    label: "压缩上下文",
    description: "压缩此对话的上下文",
    searchText: "compact compaction compress context 压缩 上下文",
  };
  expect(filterActions([action], "")).toEqual([action]);
  for (const query of ["COMPACT", "compress", "压缩", "上下文"]) {
    expect(filterActions([action], query)).toEqual([action]);
  }
  expect(filterActions([action], "skill")).toEqual([]);
  expect(filterActions([], "compact")).toEqual([]);
});

test("running a context action removes its command from the draft", () => {
  // Without this the leftover `/压缩` would be sent as a message, or appended
  // to whatever the user writes next.
  expect(removeSkillQuery("/压缩", skillQuery("/压缩", caret(3))!))
    .toEqual({ text: "", selection: caret(0) });
  expect(removeSkillQuery("请 /compress 一下", skillQuery("请 /compress 一下", caret(11))!))
    .toEqual({ text: "请 一下", selection: caret(2) });
  expect(removeSkillQuery("hi /compact", skillQuery("hi /compact", caret(11))!))
    .toEqual({ text: "hi ", selection: caret(3) });
});
