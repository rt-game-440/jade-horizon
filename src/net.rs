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

/// 参数种类：原样文本（玩家名等不可信内容，客户端不翻译不解析）。
pub const ARG_LIT: u8 = 0;
/// 参数种类：时间，`number` 为 tick。
pub const ARG_TICKS: u8 = 1;
/// 参数种类：方块或物品 id（`text`），客户端按方块、物品的顺序找本地化名称。
pub const ARG_ID: u8 = 2;
/// 参数种类：若干片段（`parts`），翻译键片段翻译、文本片段原样，空格连接。
pub const ARG_PARTS: u8 = 3;
/// 参数种类：整数（`number`），原样显示。
pub const ARG_NUMBER: u8 = 4;

/// 组合参数的一个片段：翻译键或原样文本。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetPart {
    pub translate: bool,
    pub text: String,
}

/// 一行文本的一个参数；`kind` 决定用哪个字段，其余字段留空。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetArg {
    pub kind: u8,
    pub text: String,
    pub number: i32,
    pub parts: Vec<NetPart>,
}

impl NetArg {
    pub fn lit(text: &str) -> Self {
        NetArg { kind: ARG_LIT, text: text.to_owned(), ..Default::default() }
    }

    pub fn ticks(ticks: i32) -> Self {
        NetArg { kind: ARG_TICKS, number: ticks, ..Default::default() }
    }

    pub fn id(id: &str) -> Self {
        NetArg { kind: ARG_ID, text: id.to_owned(), ..Default::default() }
    }

    pub fn number(number: i32) -> Self {
        NetArg { kind: ARG_NUMBER, number, ..Default::default() }
    }

    pub fn parts(parts: Vec<NetPart>) -> Self {
        NetArg { kind: ARG_PARTS, parts, ..Default::default() }
    }

    fn wire_size(&self) -> usize {
        1 + self.text.len() + 2 + 4 + 2 + self.parts.iter().map(|p| 1 + p.text.len() + 2).sum::<usize>()
    }
}

impl NetPart {
    pub fn key(key: &str) -> Self {
        NetPart { translate: true, text: key.to_owned() }
    }

    pub fn text(text: &str) -> Self {
        NetPart { translate: false, text: text.to_owned() }
    }
}

/// 一行由客户端翻译的文本：翻译键与结构化参数。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetLine {
    pub key: String,
    pub args: Vec<NetArg>,
}

/// 熔炉、酿造台、营火这类"输入 → 进度 → 输出"的显示。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetProgress {
    pub inputs: Vec<NetItem>,
    pub outputs: Vec<NetItem>,
    pub current: i32,
    pub total: i32,
}

/// 储罐种类：能量。
pub const TANK_ENERGY: u8 = 0;
/// 储罐种类：流体。
pub const TANK_FLUID: u8 = 1;

/// 一个能量缓冲或流体储罐：种类、流体注册名（能量为空）、当前量与容量。
/// 能量单位为 FE，流体单位为 mB。
#[derive(PayloadCodec, Clone, Debug, Default, PartialEq, Eq)]
pub struct NetTank {
    pub kind: u8,
    pub id: String,
    pub amount: i32,
    pub capacity: i32,
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
    /// 能量缓冲与流体储罐。
    pub tanks: Vec<NetTank>,
    /// 实体生命值与上限；客户端读不到生命值时用它。上限为 0 表示没有。
    pub health: f32,
    pub max_health: f32,
    /// 实体盔甲值；0 表示没有。
    pub armor: f32,
}

pub const REQUEST_ID: &str = "jade:request";
pub const DATA_ID: &str = "jade:server_data";
/// 网络协议版本；两端不一致时视同对方没装本 mod。
pub const PROTOCOL: &str = "5";

impl Request {
    /// 估算的网络字节数：目标（种类 + 4 个坐标）加 `fresh` 标记。
    pub fn wire_size(&self) -> usize {
        1 + 4 * 4 + 1
    }
}

impl ServerData {
    /// 估算的网络字节数：字符串长度加每个字段的固定开销。
    pub fn wire_size(&self) -> usize {
        let items = |v: &[NetItem]| v.iter().map(|i| i.id.len() + 4 + 2).sum::<usize>();
        let progress: usize = self.progress.iter().map(|p| items(&p.inputs) + items(&p.outputs) + 8 + 4).sum();
        let lines: usize = self.lines.iter().map(|l| l.key.len() + 2 + l.args.iter().map(NetArg::wire_size).sum::<usize>()).sum();
        let tanks: usize = self.tanks.iter().map(|t| 1 + t.id.len() + 2 + 8).sum();
        (1 + 4 * 4) + items(&self.items) + progress + lines + tanks + 12
    }
}
