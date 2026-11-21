//! 验收夹具：给原版金块一个能量缓冲（4000 / 10000 FE），让 Jade 的能量条有东西可显示。
use mod_api::{EnergyHandler, Hooks, Registrar, RegistrationContext, ServerHost, Transaction, ENERGY};

struct Battery;

impl EnergyHandler for Battery {
    fn amount(&self, _host: &mut dyn ServerHost) -> u32 {
        4000
    }
    fn capacity(&self, _host: &mut dyn ServerHost) -> u32 {
        10000
    }
    fn insert(&self, _host: &mut dyn ServerHost, _tx: &Transaction, _amount: u32) -> u32 {
        0
    }
    fn extract(&self, _host: &mut dyn ServerHost, _tx: &Transaction, _amount: u32) -> u32 {
        0
    }
}

#[derive(Default)]
struct Fixture;

#[mod_api::horizon_mod]
impl Hooks for Fixture {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let gold = r.block_key("minecraft:gold_block");
        r.block_capability(&ENERGY, &gold, |_, _, _, _| Some(Box::new(Battery)));
    }
}
