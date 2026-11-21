"""性能基线：同一次启动里用按键开关（keypad.1）切换 A（提示框开）和 B（关），比较客户端 CPU 帧耗时。

用法：python -I tools/perf.py                 对比 tests/perf_baseline.json，超阈值以非零退出
      python -I tools/perf.py --update-baseline  重写基线（阈值取本机 A/A 波动的 2 倍）
先 cargo horizon-mod build --wasm。性能测试不进 CI（CI 没有游戏环境）。

指标：
- 客户端：`horizon-ctl bench 10` 的 cpu.avg_ms / p99_ms。画面被垂直同步限在固定帧率，
  帧时间看不出 mod 开销，所以看 CPU 耗时；A 减 B 就是提示框的开销。
- 服务端：`horizon-server status --json` 里的 mspt（A、B 各取一次）。
- 网络：mod 开了 debug_stats 后每 10 秒写 perf.json（请求数、回包数、估算字节），算每秒请求数。
"""
import json
import statistics
import sys
import time
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from accept import (  # noqa: E402
    CHEST_NBT,
    CLIENT,
    FURNACE_NBT,
    HMOD,
    ROOT,
    WORK,
    Y,
    Run,
    find_game,
    launch,
    reset,
)

BASELINE = ROOT / "tests" / "perf_baseline.json"
SECONDS = 10
ROUNDS = 2  # A-B-A-B：每种状态各跑 ROUNDS 次
FLOOR_MS = 0.05  # 阈值下限：本机波动再小也不收得比这更紧
MAX_REQUESTS_PER_SECOND = 2.5
ABSOLUTE_LIMIT_MS = 1.0  # 提示框每帧最多多花这么多 CPU 时间
TOGGLE = "key.keyboard.keypad.1"

TARGETS = {
    "chest": (f"setblock -3 {Y + 1} 0 minecraft:chest[facing=east]{CHEST_NBT}", (-3, Y + 1, 0)),
    "furnace": (f"setblock 3 {Y + 1} 0 minecraft:furnace[facing=west,lit=true]{FURNACE_NBT}", (3, Y + 1, 0)),
}


def parse_json(text):
    for line in reversed(text.splitlines()):
        line = line.strip()
        if line.startswith("{"):
            try:
                return json.loads(line)
            except ValueError:
                continue
    raise RuntimeError(f"没有 JSON 输出：{text[:200]}")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    update = "--update-baseline" in sys.argv
    if not HMOD.exists():
        sys.exit("缺少 target/horizon-mod/jade.hmod，先运行 cargo horizon-mod build --wasm")
    out = ROOT / "logs" / f"perf-{datetime.now():%Y-%m-%d %H-%M-%S}"
    out.mkdir(parents=True, exist_ok=True)
    r = Run(out, "perf.log")
    game, resources = find_game()
    srv = lambda *a, **kw: r.run([game / "horizon-server.exe", "--dir", WORK / "server", *a], **kw)  # noqa: E731
    ctl = lambda *a, **kw: r.run([game / "horizon-ctl.exe", "--socket", WORK / "alice.sock", *a], **kw)  # noqa: E731

    report = {"targets": {}}
    failures = []
    try:
        launch(ctl, srv, resources, settings={"debug_stats": True})
        perf_json = WORK / "alice" / "horizon-mod-data" / "jade" / "perf.json"
        shown = True

        def set_shown(want):
            nonlocal shown
            if shown != want:
                ctl("key", TOGGLE, "tap", timeout=30)
                shown = want
                time.sleep(1.0)

        def bench():
            time.sleep(1.0)
            _, text = ctl("bench", str(SECONDS), timeout=SECONDS + 60)
            data = parse_json(text)
            _, status = srv("status", "--json", timeout=30)
            mspt = parse_json(status).get("sample", {}).get("game", {}).get("mspt", {})
            return {
                "cpu_avg": data["cpu"]["avg_ms"],
                "cpu_p99": data["cpu"]["p99_ms"],
                "fps": data["avg_fps"],
                "mspt_p50": mspt.get("p50"),
                "mspt_p99": mspt.get("p99"),
            }

        def read_net():
            try:
                return json.loads(perf_json.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                return None

        for name, (setup, (x, y, z)) in TARGETS.items():
            reset(srv)
            srv("cmd", setup)
            ctl("look-at-block", str(x), str(y), str(z), timeout=30)
            time.sleep(1.5)
            set_shown(True)
            result = {"A": [], "B": [], "AA": None}

            # A/A：同一状态连跑两次，量出本机自身的波动。
            a1, a2 = bench(), bench()
            noise = abs(a1["cpu_avg"] - a2["cpu_avg"])
            result["AA"] = noise
            result["A"] += [a1, a2]

            # 网络：A 状态下等 perf.json 刷新两次，用累计值做差。
            first = read_net()
            time.sleep(21)
            second = read_net()
            if first and second and second["now_ms"] > first["now_ms"]:
                span = (second["now_ms"] - first["now_ms"]) / 1000
                result["requests_per_second"] = (second["requests"] - first["requests"]) / span
                result["replies_per_second"] = (second["replies"] - first["replies"]) / span
                result["bytes_per_second"] = (
                    second["bytes_out"] + second["bytes_in"] - first["bytes_out"] - first["bytes_in"]
                ) / span
            else:
                failures.append(f"{name}: 没读到 perf.json（debug_stats 没生效？）")

            # A-B-A-B
            for _ in range(ROUNDS):
                set_shown(False)
                result["B"].append(bench())
                set_shown(True)
                result["A"].append(bench())
            set_shown(True)

            def med(rows, key):
                return statistics.median(row[key] for row in rows)

            result["delta_avg_ms"] = med(result["A"], "cpu_avg") - med(result["B"], "cpu_avg")
            result["delta_p99_ms"] = med(result["A"], "cpu_p99") - med(result["B"], "cpu_p99")
            result["noise_ms"] = noise
            report["targets"][name] = result
            r.note(name, json.dumps(result, ensure_ascii=False))
    except Exception as e:
        r.note("出错:", e)
        failures.append(str(e))
    finally:
        try:
            ctl("quit", timeout=60)
        except Exception as e:
            r.note("客户端退出失败:", e)
        try:
            srv("stop", timeout=120)
        except Exception as e:
            r.note("服务端停止失败:", e)

    # 对比或写基线：阈值 = 基线里的开销 + 2 倍 A/A 波动（不低于下限），另有绝对上限。
    old = json.loads(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() and not update else None
    baseline = {"max_requests_per_second": MAX_REQUESTS_PER_SECOND, "absolute_limit_ms": ABSOLUTE_LIMIT_MS, "targets": {}}
    for name, res in report["targets"].items():
        ref = old["targets"][name] if old and name in old["targets"] else None
        base_delta = ref["delta_avg_ms"] if ref else res["delta_avg_ms"]
        noise = ref["noise_ms"] if ref else res["noise_ms"]
        thr = min(base_delta + max(2 * noise, FLOOR_MS), ABSOLUTE_LIMIT_MS)
        res["threshold_ms"] = thr
        baseline["targets"][name] = {
            "delta_avg_ms": base_delta,
            "noise_ms": noise,
            "threshold_ms": thr,
            "requests_per_second": res.get("requests_per_second"),
        }
        if res["delta_avg_ms"] > thr and not update:
            failures.append(f"{name}: 提示框开销 {res['delta_avg_ms']:.3f}ms 超过阈值 {thr:.3f}ms")
        if res["delta_avg_ms"] > ABSOLUTE_LIMIT_MS:
            failures.append(f"{name}: 提示框开销 {res['delta_avg_ms']:.3f}ms 超过绝对上限 {ABSOLUTE_LIMIT_MS}ms")
        rps = res.get("requests_per_second")
        if rps is not None and rps > MAX_REQUESTS_PER_SECOND:
            failures.append(f"{name}: 每秒请求 {rps:.2f} 超过 {MAX_REQUESTS_PER_SECOND}")
    if update or not BASELINE.exists():
        if report["targets"] and not failures:
            BASELINE.write_text(json.dumps(baseline, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            r.note("基线已写入", BASELINE)
        else:
            r.note("有失败，不写基线")

    lines = [f"# 性能报告 {datetime.now():%Y-%m-%d %H:%M:%S}", "", "单位 ms；开销 = 提示框开(A) 减 关(B) 的 CPU 平均帧耗时。", ""]
    lines += ["| 目标 | A/A 波动 | 开销(avg) | 开销(p99) | 阈值 | 请求/s | 回包/s | 字节/s |", "|---|---|---|---|---|---|---|---|"]
    for name, res in report["targets"].items():
        lines.append(
            f"| {name} | {res['AA']:.3f} | {res['delta_avg_ms']:.3f} | {res['delta_p99_ms']:.3f} | "
            f"{res['threshold_ms']:.3f} | {res.get('requests_per_second', float('nan')):.2f} | "
            f"{res.get('replies_per_second', float('nan')):.2f} | {res.get('bytes_per_second', float('nan')):.0f} |"
        )
    lines += ["", "服务端 mspt（A / B 最后一次）：", ""]
    for name, res in report["targets"].items():
        a, b = res["A"][-1], res["B"][-1]
        lines.append(f"- {name}: A p50 {a['mspt_p50']} p99 {a['mspt_p99']}；B p50 {b['mspt_p50']} p99 {b['mspt_p99']}")
    lines += ["", "结论：" + ("通过" if not failures else "失败")]
    lines += [f"- {f}" for f in failures]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (out / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    r.note("报告:", out / "report.md")
    print("\n".join(lines))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
