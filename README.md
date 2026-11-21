# Jade for Horizon

[Jade](https://github.com/Snownee/Jade)（作者 Snownee）在 [Horizon](https://playhorizon.dev)（Rust 重写的 Minecraft 26.3）上的非官方 WebAssembly 移植。
准星对准方块或实体时，在屏幕顶部显示它的信息。

本项目不是 Snownee 的官方作品，也未获原作者背书。

## 功能

只装客户端即可：

- 名称、图标、所属模组（ObjectName / ModName）
- 生物生命值（心形图标，最大生命超过 20 时显示数字）
- 作物生长进度、中继器延迟、比较器模式、拉杆状态、红石强度、音符盒音色与音高、蜂巢蜂蜜等级、不稳定 TNT
- 挖掘信息：挖这个方块要什么工具，手上的工具能不能采下掉落物
- 流体：准星对准水、岩浆等流体时显示名称与水桶图标（与 Jade 一样默认开启）
- 按住 Shift 看详细信息：容器内容默认只显示一行，繁殖冷却、成长时间等长尾信息归到详细模式

服务端也安装后额外显示：

- 容器内容（通过 `horizon:storage` 能力读取，原版容器与注册了该能力的模组容器都支持）、战利品未生成、容器已上锁
- 熔炉 / 高炉 / 烟熏炉进度，酿造台进度与燃料，营火烹饪进度
- 蜂巢蜜蜂数、刷怪笼生物与冷却、唱片机唱片、讲台书本、物品展示框物品
- 幼年生物成长时间、繁殖冷却、求爱状态、鸡下一次下蛋时间

服务端只回应 16 格内、同一维度的目标。本 mod 带内容（`side = "both"`），Horizon 的 mod 握手要求服务端和客户端装同一批 mod，
只装一边的组合会被直接拒绝进服（`Your mods don't match this server's`），所以没有“只装客户端 / 只装服务端”的验收场景。

## 设置与按键

游戏里“选项 > 模组 > Jade”：

- 总开关、锚点（顶部居中 / 左上 / 右上 / 左中 / 右中）、水平与垂直偏移、背景不透明度
- 主题：原版风、深色、浅色、高对比度（文字与背景对比度不低于 4.5:1）
- 动画：切换目标时淡入淡出、尺寸平滑过渡
- 分项开关：生命、盔甲、药水、马属性、村民、主人、容器内容、熔炉进度、作物、红石、挖掘工具与进度、流体、掉落物、模组名
- 调试统计：打开后每 10 秒把网络计数写进 `horizon-mod-data/jade/perf.json`（给 `tools/perf.py` 用，默认关）

“控制”里有两个按键：`显示/隐藏提示框`（默认小键盘 1，运行期开关，重启恢复）和 `切换流体显示`（默认小键盘 2）。
提示框关闭时不组装内容也不发请求。缩放没有做：Horizon 的 `DrawContext` 没有缩放，物品图标也不能带位姿绘制。

## 构建

```bash
cargo horizon-mod build --wasm
```

产物在 `target/horizon-mod/jade.hmod`。放进游戏目录的 `horizon-mods/`，并在 `horizon-mods.txt` 里加一行 `mod.jade:true`。

## 检查

```bash
cargo test
```

```bash
cargo horizon-mod check
```

`cargo test` 包含翻译键检查：代码里用到的每个 Jade 翻译键都必须在 `en_us.json` 和 `zh_cn.json` 里存在。
`tools/api.py <正则>` 可以在 SDK 的 `surface.json` 里查 API 签名。

## 游戏内验收

需要已安装 Horizon 启动器（并且没开 No AI）。脚本会构建一个装了 jade 的专用服务端，
再离屏启动一个客户端进服，在 y=150 搭平台、逐个场景摆方块和生物，对准后截图，
裁出顶部的 Jade 提示框并与 `tests/baselines/` 里的基准图逐像素比对：

```powershell
python -I tools/accept.py                      # 比对全部场景
python -I tools/accept.py --only chest          # 只跑指定场景（可重复给）
python -I tools/accept.py --update              # 重写全部基准
python -I tools/accept.py --update --only chest # 只重写受影响的场景
```

场景可以带设置（`settings=`）、主题和锚点；设置相同的场景共用一次客户端启动，设置在启动前写进设置文件。
每批结束会扫一遍服务端和客户端日志里 jade 相关的 panic / 错误。

窗口固定 1280×720。截图前一刻 `/tick freeze`、截完立即 `/tick unfreeze`，
并把场景里的动态值（熔炉进度、生物成长时间）在冻结后固定，所以同一场景可以反复复现。
比对规则：任一通道相差超过 32 的像素超过 0.1%（或尺寸不同）判为失败；
失败时 `logs/accept-<时间>/` 下有 `<场景>-actual.png`、`<场景>-diff.png`（差异标红）和 `report.txt`，
全部场景的提示框裁剪图是 `<场景>.png`。改动显示内容后用 `--update --only` 只更新受影响的场景。

需要人工看图时用 `tools/verify.py`，它启动默认实例的测试世界 `jade-verify`，逐个对准并截整屏：

```powershell
python -I tools/verify.py            # 加 --no-deploy 跳过部署，--keep-open 结束后不退出游戏
pwsh tools/contact_sheet.ps1         # 把最新一次截图的 HUD 区域拼成 hud-sheet.png
```

输出在 `logs/verify-<时间>/`：每个目标一张截图、`verify.log`（全部 MCP 调用与返回）、`game-log-jade.txt`（游戏日志中与 jade/wasm/panic 相关的行）。
`tools/hzmcp.py` 是驱动 `horizon-launcher mcp` 的小客户端，也可单独用：`python -I tools/hzmcp.py list game_`。

### 多人 e2e

```powershell
python -I tools/e2e_mp.py            # 加 --keep 结束后保留服务端和客户端，便于手动排查
```

用启动器已安装版本里的 `horizon-server.exe` 在 `target/e2e/server` 建离线服务端（端口 25599，装 jade），
再用 `horizon-ctl` 离屏启动 Alice、Bob 两个客户端（各自独立的游戏目录 `target/e2e/<名字>`，`--dev-mod` 加载 jade）进服。
进服后由客户端搭平台、传送，两人互相瞄准截图，并对照看箱子（服务端容器数据）和幼年猪（成长时间）。
每张截图会自动检测顶部有没有 Jade 提示框，全部通过时退出码为 0；输出在 `logs/e2e-<时间>/`。
不会动你自己的游戏目录和存档。

### 性能基线

```powershell
python -I tools/perf.py                    # 对比 tests/perf_baseline.json，超阈值以非零退出
python -I tools/perf.py --update-baseline  # 在本机重写基线
```

同一次启动里用小键盘 1 切换提示框的开（A）和关（B），对准箱子和熔炉各按 A-B-A-B 跑 `horizon-ctl bench 10`，
比较 CPU 平均耗时（画面被垂直同步限速，帧时间看不出 mod 开销）。先跑 A/A 量出本机波动，阈值是“基线开销 + 2 倍波动”，
绝对上限 1ms；同时用 `perf.json` 算每秒请求数（上限 2.5），并记录服务端 mspt。报告在 `logs/perf-<时间>/report.md`。
性能测试不放进 CI（CI 没有游戏环境），换机器后请重写基线。

## 许可与署名

- 原作：Jade © Snownee，[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/)。
- 本移植用 Rust 在 Horizon mod API 上重写了 Jade 的部分功能，界面布局与显示内容参照原作；
  `assets/jade/lang/` 下的语言文件直接取自 Jade（26.3-neoforge 分支），翻译归 Jade 及其译者所有。
- 本项目同样以 CC BY-NC-SA 4.0 发布（见 `LICENSE.md`），不得用于商业目的。
