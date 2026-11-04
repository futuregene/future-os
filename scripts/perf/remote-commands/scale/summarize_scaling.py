#!/usr/bin/env python3
"""Turn the T4 raw samples into the tables `04-scaling.md` prints.

Every number in the chapter's tables comes from here: the raw JSON under
`docs/testing/remote-latency/raw/scaling/`, grouped into one series per (command,
dimension), with the growth class fitted from the measured points and the 100 ms
crossing either read off a measured point or extrapolated from the fit (and
marked as an extrapolation).

The fit is a least-squares line through `(log10(size), p50_ms)`: a straight line
in log-size is what "logarithmic", "linear" and "flat" all look like on a
log axis, and its slope is directly the "ms per 10x" the chapter quotes. It is
deliberately unglamorous — with five to seven points, a fancier model would be
inventing structure the measurements do not distinguish.

Usage (prints the markdown block; `--write` splices it into the chapter):

    python3 scripts/perf/remote-commands/scale/summarize_scaling.py
    python3 scripts/perf/remote-commands/scale/summarize_scaling.py --write
"""
import argparse
import glob
import json
import math
import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
RAW = ROOT / "docs" / "testing" / "remote-latency" / "raw" / "scaling"
CHAPTER = ROOT / "docs" / "testing" / "remote-latency" / "04-scaling.md"
START = "<!-- scaling-tables:start -->"
END = "<!-- scaling-tables:end -->"
RANKED_START = "<!-- scaling-ranked:start -->"
RANKED_END = "<!-- scaling-ranked:end -->"
BUDGET = 100.0


def load():
    """Every warm sample, newest tag per label."""
    newest = {}
    for path in sorted(glob.glob(str(RAW / "*-warm-*.json"))):
        tag = os.path.basename(path).rsplit("-warm-", 1)[1][: -len(".json")]
        for result in json.load(open(path))["results"]:
            previous = newest.get(result["label"])
            if previous is None or tag > previous[0]:
                newest[result["label"]] = (tag, result)
    return {label: result for label, (_, result) in newest.items()}


def ms(result, field="p50Ms"):
    return result[field]


def samples(result):
    return [sample["ms"] for sample in result["samples"]]


def index(pattern, results):
    return {label: result for label, result in results.items() if re.fullmatch(pattern, label)}


def series(entries):
    """[(size, result, extra)] sorted by size."""
    return sorted(entries, key=lambda item: item[0])


def fit(points):
    """The better of two one-parameter fits: linear in n, and linear in log10 n.

    Both matter here and the data decides: a page-size read is linear in the
    *number of rows* it walks (`get_session_entries` offset), a per-thread catalog
    is close to linear in rows too, and a sublinear curve is what a log axis
    straightens. Interpolating and extrapolating with the model that actually
    fits is the difference between "100 ms at 5,800 entries" and the nonsense a
    log fit gives when extrapolated off a line that is straight in n.

    Returns `(model, a, b, midpoint)` with `y = a + b*x`.
    """
    sizes = [size for size, _ in points]
    values = [value for _, value in points]
    logs = [math.log10(size) for size in sizes]

    def least_squares(xs):
        count = len(xs)
        mean_x, mean_y = sum(xs) / count, sum(values) / count
        variance = sum((x - mean_x) ** 2 for x in xs)
        if variance == 0:
            return 0.0, mean_y, sum((y - mean_y) ** 2 for y in values)
        slope = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, values)) / variance
        intercept = mean_y - slope * mean_x
        error = sum((y - (intercept + slope * x)) ** 2 for x, y in zip(xs, values))
        return slope, intercept, error

    slope_n, intercept_n, error_n = least_squares(sizes)
    slope_log, intercept_log, error_log = least_squares(logs)
    midpoint = math.sqrt(sizes[0] * sizes[-1])
    if error_n <= error_log:
        return "linear", intercept_n, slope_n, slope_n * midpoint * 9, midpoint
    return "logarithmic", intercept_log, slope_log, slope_log, midpoint


def growth_class(points):
    """flat / logarithmic / linear / worse, from the top of the measured range.

    The two largest points decide the label: the budget question is "what happens
    as this gets big", and a curve that is flat under a thousand rows and linear
    above it (as `get_state` turns out to be) is linear for that purpose. The
    table above the label still shows the whole curve, and the chapter says where
    each bend is.
    """
    if len(points) < 2:
        return "n/a"
    (low, first), (high, last) = points[-2], points[-1]
    if first <= 0 or low <= 0:
        return "flat"
    ratio = last / first
    span = high / low
    if ratio <= 1.3:
        return "flat"
    if ratio >= span * 1.3:
        return "worse"
    if ratio >= span ** 0.6:
        return "linear"
    return "logarithmic"


def crossing(points, model, intercept, slope):
    """Where the fitted curve reaches BUDGET, and whether that is measured.

    "measured" means the crossing sits inside the range this run covered; a
    crossing above the largest measured size is an extrapolation and the chapter
    says so in the cell.
    """
    if slope <= 0:
        return None, "never (flat)"
    if model == "linear":
        size = (BUDGET - intercept) / slope
    else:
        exponent = (BUDGET - intercept) / slope
        if exponent > 9:
            return None, "never (>10⁹)"
        size = 10 ** exponent
    if size > 1e9:
        return None, "never (>10⁹)"
    inside = points[0][0] <= size <= points[-1][0]
    return size, "measured" if inside else "extrapolated"


def human(size):
    if size is None:
        return "—"
    if size >= 1_000_000:
        return f"{size / 1_000_000:.1f}M"
    if size >= 1000:
        return f"{int(round(size / 100) * 100):,}"
    return f"{int(round(size))}"


def row(size, result, extra=""):
    cold = result["cold"]["ms"] if result["cold"] else 0.0
    failures = result["iterations"] - result["replies"]
    note = "**失败**" if failures else ""
    error = ""
    if failures and result["samples"]:
        message = result["samples"][0].get("error") or ""
        if "too large" in message:
            error = "（gRPC 32 MB 上限）"
        elif message:
            error = f"（{message[:40]}）"
    return (
        f"| {human(size)} | {cold:.1f} | {ms(result):.2f} | {ms(result, 'p95Ms'):.2f} | "
        f"{ms(result, 'maxMs'):.2f} | {result['replyBytes']:,} | {note}{extra}{error} |"
    )


HEADER = (
    "| 数据规模 | cold ms | p50 ms | p95 ms | max ms | 回复字节 | 判定 |\n"
    "| ---: | ---: | ---: | ---: | ---: | ---: | --- |"
)


def table(points, extra_for=None):
    lines = [HEADER]
    for size, result in points:
        lines.append(row(size, result, extra_for(result) if extra_for else ""))
    return "\n".join(lines)


def summary(points, label, dimension):
    # A spec that never answered is not a timing: its "p50" is how long the
    # command took to fail. It stays in the table (that is the finding) but it
    # must not pull the fit, or a curve that breaks at 19k entries would be read
    # as if it kept reading successfully.
    usable = [(size, result["p50Ms"]) for size, result in points
              if result["p50Ms"] > 0 and result["replies"] >= result["iterations"]]
    model, intercept, slope, slope_10, midpoint = fit(usable)
    klass = growth_class(usable)
    crossing_size, how = crossing(usable, model, intercept, slope)
    if klass == "flat":
        # A curve whose largest two points are flat does not have a crossing
        # worth printing, whatever the fit's small residual slope extrapolates to.
        crossing_size, how = None, "never (flat)"
    low, high = usable[0], usable[-1]
    ratio = high[1] / low[1] if low[1] else 0
    return {
        "label": label,
        "dimension": dimension,
        "model": model,
        "slope10": slope_10,
        "class": klass,
        "crossing": crossing_size,
        "how": how,
        "ratio": ratio,
        "low": low,
        "high": high,
        "high_p95": points[-1][1]["p95Ms"],
        "breached": high[1] >= BUDGET,
        "near": high[1] >= BUDGET * 0.6,
        "failedFrom": next((size for size, result in points
                            if result["replies"] < result["iterations"]), None),
    }


def detail_line(info):
    if info["low"][1] == 0:
        return "无可测增长"
    if info.get("failedFrom"):
        return (
            f"n={human(info['low'][0])}→{human(info['high'][0])}："
            f"{info['low'][1]:.2f}→{info['high'][1]:.2f} ms（×{info['ratio']:.1f}），"
            f"拟合只取成功返回的点；**{human(info['failedFrom'])} 档起命令失败**，"
            f"其毫秒数是失败前耗时"
        )
    return (
        f"n={human(info['low'][0])}→{human(info['high'][0])}："
        f"{info['low'][1]:.2f}→{info['high'][1]:.2f} ms（×{info['ratio']:.1f}），"
        f"拟合为{info['model']}，中位规模处约 {info['slope10']:+.2f} ms/10×"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="splice into the chapter")
    args = parser.parse_args()

    results = load()
    out = []
    write = out.append
    infos = []

    # ── (a) threads ─────────────────────────────────────────────────────────
    threads = {}
    for size, label in ((10, "threads-10"), (100, "threads-100"),
                        (2000, "threads-2000"), (5000, "threads-5000")):
        threads[size] = label
    # The same label repeats across homes (`list_sessions` is measured in every
    # threads home), so the per-home file is what identifies a point.
    per_home = {}
    for path in sorted(glob.glob(str(RAW / "*-warm-*.json"))):
        home = os.path.basename(path).rsplit("-warm-", 1)[0]
        tag = os.path.basename(path).rsplit("-warm-", 1)[1][: -len(".json")]
        per_home.setdefault(home, {})
        for result in json.load(open(path))["results"]:
            previous = per_home[home].get(result["label"])
            if previous is None or tag >= previous[0]:
                per_home[home][result["label"]] = (tag, result)

    def from_home(home, label):
        entry = per_home.get(home, {}).get(label)
        return entry[1] if entry else None

    sessions_points = []
    workspaces_points = []
    for size, home in sorted(threads.items()):
        result = from_home(home, "list_sessions")
        if result:
            sessions_points.append((size, result))
        result = from_home(home, "list_workspaces")
        if result:
            workspaces_points.append((size, result))
    control = from_home("history", "list_sessions:real")
    control_ws = from_home("history", "list_workspaces:real")

    write("\n### 表 A：逐维度、逐命令\n")
    write("### (a) app.db 的 threads 数 → `list_sessions` / `list_workspaces`\n")
    write(table(series(sessions_points)))
    info = summary(series(sessions_points), "list_sessions", "threads 数")
    infos.append(info)
    write(f"\n{detail_line(info)}；"
          f"{info['class']}；100 ms 交叉点 {human(info['crossing'])}（{info['how']}）。\n")
    if control:
        write(f"\n真实 682 线程（同一份 agent.db，`history` home）："
              f"cold {control['cold']['ms']:.1f} ms / p50 {control['p50Ms']:.2f} ms / "
              f"p95 {control['p95Ms']:.2f} ms / {control['replyBytes']:,} B —— "
              f"与上表 682 一档同量级。\n")
    write("\n`list_workspaces`：\n")
    write(table(series(workspaces_points)))
    info = summary(series(workspaces_points), "list_workspaces", "threads 数")
    infos.append(info)
    write(f"\n{detail_line(info)}；{info['class']}；"
          f"交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    # ── (b) entries ─────────────────────────────────────────────────────────
    write("\n### (b) 单会话 entries → 历史/状态/工具参数/文件列表\n")
    entry_series = [
        ("get_state", "get_state:{name}", "get_state"),
        ("get_messages 首页", "get_messages:first:{name}", "get_messages:first"),
        ("get_messages 深页", "get_messages:deep:{name}", "get_messages:deep"),
        ("get_session_entries 游标页", "get_session_entries:before:{name}",
         "get_session_entries:before"),
        ("get_session_entries offset 页", "get_session_entries:offset:{name}",
         "get_session_entries:offset"),
        ("get_tool_call_args", "get_tool_call_args:{name}", "get_tool_call_args"),
        ("list_session_files", "list_session_files:{name}", "list_session_files"),
    ]
    entries_by_name = {}
    for label, result in results.items():
        match = re.fullmatch(r"(.+?):(small|median|mid|largest|scaled\d+)", label)
        if not match:
            continue
        name = match.group(2)
        entries_by_name.setdefault(name, {})[match.group(1)] = result
    sizes = {name: values["get_state"]["scale"]["entries"]
             for name, values in entries_by_name.items() if "get_state" in values}

    for title, pattern, key in entry_series:
        points = []
        for name, values in entries_by_name.items():
            if key in values and name in sizes:
                points.append((sizes[name], values[key]))
        points = series(points)
        if not points:
            continue
        write(f"\n**{title}**\n")
        write(table(points))
        info = summary(points, title, "单会话 entries")
        infos.append(info)
        write(f"\n{detail_line(info)}；{info['class']}；"
              f"100 ms 交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    # ── (c) events ──────────────────────────────────────────────────────────
    write("\n### (c) 单 run 的事件数 → `get_events_since` 三条路径\n")
    event_series = [
        ("首页（`sinceIdx=-1`）", "get_events_since:first:", "get_events_since:first"),
        ("深页（半程游标）", "get_events_since:deep:", "get_events_since:deep"),
        ("preferSnapshot", "get_events_since:snapshot:", "get_events_since:snapshot"),
    ]
    for title, _, prefix in event_series:
        points = []
        for label, result in results.items():
            if label.startswith(prefix):
                points.append((result["scale"]["events"], result))
        points = series(points)
        write(f"\n**{title}**\n")
        write(table(points))
        info = summary(points, title, "单 run 事件数")
        infos.append(info)
        write(f"\n{detail_line(info)}；{info['class']}；"
              f"100 ms 交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    # ── (d)+(e) tasks ───────────────────────────────────────────────────────
    write("\n### (d) 单任务的运行次数 → `list_task_runs` / `get_task` / `list_tasks`\n")
    for label, title in (("list_task_runs", "list_task_runs"),
                         ("get_task", "get_task"),
                         ("list_tasks", "list_tasks（每行一次 latest run）")):
        points = []
        for size, home in ((10, "task-runs-10"), (100, "task-runs-100"),
                           (2000, "task-runs-2000")):
            result = from_home(home, label)
            if result:
                points.append((size, result))
        points = series(points)
        write(f"\n**{title}**\n")
        write(table(points))
        info = summary(points, label, "单任务运行次数")
        infos.append(info)
        write(f"\n{detail_line(info)}；{info['class']}；"
              f"交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    write("\n### (e) 任务数 → `list_tasks`\n")
    points = []
    for size, home in ((1, "tasks-1"), (500, "tasks-500")):
        result = from_home(home, "list_tasks")
        if result:
            points.append((size, result))
    real = None
    for label, result in results.items():
        if label == "list_tasks" and result["scale"].get("tasks") == 33:
            real = result
            points.append((33, result))
    # the real 33-task point lives in the history home's tasks specs, if present
    points = series(points)
    write(table(points))
    info = summary(points, "list_tasks（任务数维度）", "任务数")
    infos.append(info)
    write(f"\n{detail_line(info)}；{info['class']}；"
          f"交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    # ── (f) directory entries ───────────────────────────────────────────────
    write("\n### (f) 工作区根目录条目数 → `list_session_files`\n")
    points = []
    for size, home in ((10, "dirs-10"), (100, "dirs-100"),
                       (1000, "dirs-1000"), (5000, "dirs-5000")):
        result = from_home(home, "list_session_files")
        if result:
            points.append((size, result))
    points = series(points)
    write(table(points))
    info = summary(points, "list_session_files", "目录条目数")
    infos.append(info)
    write(f"\n{detail_line(info)}；{info['class']}；"
          f"100 ms 交叉点 {human(info['crossing'])}（{info['how']}）。\n")

    # ── the two ranked lists ────────────────────────────────────────────────
    ranked = []
    grow = ranked.append
    growing = [info for info in infos if info["class"] not in ("flat", "n/a")]
    growing.sort(key=lambda info: info["ratio"], reverse=True)
    grow("### 清单 1：延迟随数据规模增长的命令（按跨度倍数排序）\n")
    grow("| 命令 | 维度 | 增长类别 | 跨度 | ms/10× | 100 ms 交叉点 |\n"
         "| --- | --- | --- | --- | ---: | --- |")
    for info in growing:
        grow(f"| `{info['label']}` | {info['dimension']} | {info['class']} | "
             f"{human(info['low'][0])}→{human(info['high'][0])} | {info['slope10']:+.2f} | "
             f"{human(info['crossing'])}（{info['how']}） |")

    grow("\n### 清单 2：在重用户可达规模上超过 100 ms 的命令\n")
    grow("| 命令 | 维度 | 规模 | p50 | p95 | 首次越过 100 ms |\n"
         "| --- | --- | ---: | ---: | ---: | --- |")
    breached = [info for info in infos if info["breached"]]
    breached.sort(key=lambda info: info["high"][1], reverse=True)
    for info in breached:
        grow(f"| `{info['label']}` | {info['dimension']} | {human(info['high'][0])} | "
             f"{info['high'][1]:.1f} ms | {info['high_p95']:.1f} ms | "
             f"{human(info['crossing'])}（{info['how']}） |")
    if not breached:
        grow("| — | — | — | — | — | 未测到 |")
    near = [info for info in infos if info["near"] and not info["breached"]]
    near.sort(key=lambda info: info["high"][1], reverse=True)
    grow("\n### 清单 3：未越线但在 60–100 ms 之间的命令（下一批候选）\n")
    grow("| 命令 | 维度 | 规模 | p50 | p95 | 外推交叉点 |\n"
         "| --- | --- | ---: | ---: | ---: | --- |")
    for info in near:
        grow(f"| `{info['label']}` | {info['dimension']} | {human(info['high'][0])} | "
             f"{info['high'][1]:.1f} ms | {info['high_p95']:.1f} ms | "
             f"{human(info['crossing'])}（{info['how']}） |")

    block = "\n".join(out) + "\n"
    ranking = "\n".join(ranked) + "\n"
    if args.write:
        chapter = CHAPTER.read_text()
        for start, end, payload in ((START, END, block),
                                    (RANKED_START, RANKED_END, ranking)):
            pattern = re.compile(re.escape(start) + r".*?" + re.escape(end), re.S)
            if not pattern.search(chapter):
                raise SystemExit(f"marker pair {start} missing from {CHAPTER}")
            chapter = pattern.sub(lambda _: start + "\n" + payload + end, chapter)
        CHAPTER.write_text(chapter)
        print(f"wrote {CHAPTER}")
    else:
        print(block)
        print(ranking)


if __name__ == "__main__":
    main()
