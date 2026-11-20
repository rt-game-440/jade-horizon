# Jade for Horizon

[Jade](https://github.com/Snownee/Jade)（作者 Snownee）在 [Horizon](https://playhorizon.dev)（Rust 重写的 Minecraft 26.3）上的非官方 WebAssembly 移植。
准星对准方块或实体时，在屏幕顶部显示它的信息。

本项目不是 Snownee 的官方作品，也未获原作者背书。

## 功能

只装客户端即可：

- 名称、图标、所属模组（ObjectName / ModName）
- 生物生命值（心形图标，最大生命超过 20 时显示数字）
- 作物生长进度、中继器延迟、比较器模式、拉杆状态、红石强度、音符盒音色与音高、蜂巢蜂蜜等级、不稳定 TNT

服务端也安装后额外显示：

- 容器内容（通过 `horizon:storage` 能力读取，原版容器与注册了该能力的模组容器都支持）、战利品未生成、容器已上锁
- 熔炉 / 高炉 / 烟熏炉进度，酿造台进度与燃料，营火烹饪进度
- 蜂巢蜜蜂数、刷怪笼生物与冷却、唱片机唱片、讲台书本、物品展示框物品
- 幼年生物成长时间、繁殖冷却、求爱状态、鸡下一次下蛋时间

服务端只回应 16 格内、同一维度的目标。网络通道为可选，对方没装本 mod 时照常进服。

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

需要已安装 Horizon 启动器（并且没开 No AI）。脚本会把 `target/horizon-mod/jade.hmod` 部署到默认实例的
`horizon-mods/` 并在 `horizon-mods.txt` 中启用，离屏启动测试世界 `jade-verify`，在 y=150 搭平台、放置熔炉、
箱子、小麦、比较器、音符盒、酿造台和两只生物，逐个对准后截图：

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

## 许可与署名

- 原作：Jade © Snownee，[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/)。
- 本移植用 Rust 在 Horizon mod API 上重写了 Jade 的部分功能，界面布局与显示内容参照原作；
  `assets/jade/lang/` 下的语言文件直接取自 Jade（26.3-neoforge 分支），翻译归 Jade 及其译者所有。
- 本项目同样以 CC BY-NC-SA 4.0 发布（见 `LICENSE.md`），不得用于商业目的。
