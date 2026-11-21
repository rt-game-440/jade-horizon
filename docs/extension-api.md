# 第三方 mod 扩展接口调研（#15）

调研对象：Horizon SDK `5ef7734`（`.horizon/sdk/api/crates/horizon-mod-vocab`、`horizon-mod-api`）。
目标：让其他 mod 为自己的方块或实体在 Jade 提示框里追加文本行或物品行。

## 结论

1. **先做“标准能力”**：服务端已经读 `horizon:storage` 显示容器内容，同样的方式再读 `horizon:energy`、`horizon:fluids`，
   注册了这些能力的 mod 方块就能显示能量和储罐，对方不用写任何 Jade 相关代码。这也是原版 Jade 的
   EnergyStorageProvider / FluidStorageProvider 做的事。
2. **再做数据驱动插件**：其他 mod 在自己的数据包里放 JSON，声明“某个方块或实体显示哪些字段”，Jade 用 reload listener 读取。
   这是目前唯一不依赖 wasm 之间直接调用、又能让对方自定义显示内容的方式。
3. **wasm 之间直接调用（Java 版 Jade 插件的做法）暂不可行**，见下文“排除的方案”。等 Horizon 给出跨 wasm mod 的接口后再看。

## 可用的机制

### 标准能力（推荐，先做）

SDK 内置的块能力名：`horizon:items`、`horizon:storage`、`horizon:fluids`、`horizon:energy`（另有物品、实体上的版本）。
服务端按坐标查能力就能读到模组方块的内容，与 `server.rs` 现在读容器的方式一样。

- 优点：对方零改动，只要按 Horizon 的规范注册了能力；协议里加两类行即可。
- 缺点：只能显示这几类通用数据。

### 数据驱动插件（推荐，第二步）

`Hooks::add_server_reload_listeners`（服务端）和 `add_client_reload_listeners`（客户端）可以注册监听器，
在开服、`/reload` 时读取**所有命名空间**下指定目录的文件（`data/*/<目录>/`、`assets/*/<目录>/`），
数据包或资源包也能覆盖。草案：

```text
data/<mod>/jade_providers/<名字>.json   服务端：从方块实体或实体的 NBT 里取值
assets/<mod>/jade_providers/<名字>.json 客户端：从方块状态属性取值（只装客户端时也能显示）
```

```json
{
  "targets": ["mymod:crusher"],
  "lines": [
    { "key": "mymod.jade.speed", "nbt": "Speed" },
    { "key": "mymod.jade.mode", "state": "mode", "detail": true },
    { "items": "Inventory" }
  ]
}
```

- `key` 是对方 mod 自带的翻译键，参数是取到的值；`detail` 表示只在按住 Shift 时显示。
- 服务端行沿用现有的 `NetLine`（翻译键加参数），不用改协议；物品行沿用 `NetItem`。
- 优点：跨 wasm、跨语言都可用，对方只需要加数据文件；服主也能用数据包给没适配的 mod 补规则。
- 缺点：只能表达“取字段 → 翻译显示”，没法做计算；需要限制每个目标的行数和 NBT 路径深度，避免拖慢服务端。

## 排除的方案

| 方案 | 为什么不行 |
| --- | --- |
| 对方 mod 依赖 Jade 发布的 `[api]` crate，注册自定义能力，Jade 按能力查询 | SDK 只在同一组 mod 内能按 handler 类型向下转型；`parts/capabilities.rs` 的 `OwnProvider` 注释说明 wasm mod 的 provider 对其他组是宿主代理，查询拿不到。跨 wasm 是否可行要等 Horizon 明确 |
| 对方往 `jade:*` 网络包里发数据 | 网络包 id 带命名空间，推测别的 mod 不能注册 Jade 命名空间下的包（未验证） |
| 约定在方块实体 NBT 里写 `jade` 字段 | 会写进存档，要求对方改存储格式，也没有翻译和排版信息 |
| 事件或钩子让其他 mod 往提示框加行 | 在 SDK 里没找到自定义事件或 mod 之间互相派发钩子的接口 |

## 建议的后续 issue

1. ~~服务端读取 `horizon:energy`、`horizon:fluids`，显示能量条和储罐~~ 已完成（#27）。能量用 `tests/fixture-energy` 夹具 mod（给原版金块一个能量缓冲）验收，流体用炼药锅验收。
2. 数据驱动插件：先做服务端 NBT 版，规则格式就是上面的草案，再写一个示例数据包和文档。
3. 关注 Horizon 是否开放跨 wasm mod 的 `[api]` 能力或 mod 间消息；开放后再设计代码级插件接口。
