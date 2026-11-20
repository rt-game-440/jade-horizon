//! 客户端与服务端之间的网络包：客户端说"我在看什么"，服务端回"那里有什么"。
use mod_api::PayloadCodec;

/// 目标种类：方块。
pub const KIND_BLOCK: u8 = 1;
/// 目标种类：实体。
pub const KIND_ENTITY: u8 = 2;

/// 一个被看着的目标，方块用坐标，实体用网络 id。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetTarget {
    pub kind: u8,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub entity: i32,
}

/// 客户端 → 服务端：请求目标的服务端数据。
#[derive(PayloadCodec, Clone, Debug, Default)]
pub struct Request {
    pub target: NetTarget,
    /// 客户端刚换到这个目标：服务端即使数据没变也要回包。
    pub fresh: bool,
}

/// 一格物品：物品 id 与数量。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetItem {
    pub id: String,
    pub count: i32,
}

/// 一行由客户端翻译的文本：翻译键与参数。参数以 `@` 开头时它本身也是翻译键。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetLine {
    pub key: String,
    pub args: Vec<String>,
}

/// 熔炉、酿造台、营火这类"输入 → 进度 → 输出"的显示。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetProgress {
    pub inputs: Vec<NetItem>,
    pub outputs: Vec<NetItem>,
    pub current: i32,
    pub total: i32,
}

/// 服务端 → 客户端：目标的服务端数据。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq)]
pub struct ServerData {
    pub target: NetTarget,
    /// 容器内容（已合并同类）。
    pub items: Vec<NetItem>,
    /// 容器被锁或战利品未生成等情况。
    pub progress: Vec<NetProgress>,
    pub lines: Vec<NetLine>,
    /// 实体生命值与上限；客户端读不到生命值时用它。上限为 0 表示没有。
    pub health: f32,
    pub max_health: f32,
}

pub const REQUEST_ID: &str = "jade:request";
pub const DATA_ID: &str = "jade:server_data";
/// 网络协议版本；两端不一致时视同对方没装本 mod。
pub const PROTOCOL: &str = "2";
