//! Jade for Horizon：准星对准方块或实体时，在屏幕顶部显示它的信息。
//!
//! 移植自 Snownee 的 Jade（https://github.com/Snownee/Jade），CC BY-NC-SA 4.0。
//! 客户端单独安装即可显示名称、模组、生命值、作物生长等；服务端也安装时，
//! 额外显示容器内容、熔炉进度等需要服务端数据的信息。
use std::sync::OnceLock;

use mod_api::{
    self, ClientHost, ClientTickPostEvent, GuiRenderPostEvent, Hooks, HudMode, PayloadId, PlayerLoggedInEvent, PlayerLoggedOutEvent,
    Registrar, RegistrationContext, ServerHost, ToClient, ToServer,
};

mod client;
mod dropped;
mod fluid;
mod harvest;
#[cfg(test)]
mod lang_keys_test;
mod limit;
mod net;
mod render;
mod server;

use net::{DATA_ID, PROTOCOL, REQUEST_ID, Request, ServerData};

static TO_SERVER: OnceLock<ToServer<Request>> = OnceLock::new();
static TO_CLIENT: OnceLock<ToClient<ServerData>> = OnceLock::new();

#[derive(Default)]
struct Jade;

#[mod_api::horizon_mod]
impl Hooks for Jade {
    fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
        for package in cx.mods().iter() {
            for ns in package.namespaces() {
                client::set_mod_name(ns, package.name());
            }
            client::set_mod_name(package.id(), package.name());
        }
        let mut payloads = r.payload_handlers();
        // optional：对方没装本 mod 时照常进服，只是没有服务端数据。
        let mut view = payloads.registrar(PROTOCOL).optional();
        let to_server = view.play_to_server_typed::<Request>(PayloadId(REQUEST_ID.into()), |request, cx| {
            let player = cx.player().map_err(|e| format!("{e:?}"))?;
            let data = {
                let host = cx.server_host().map_err(|e| format!("{e:?}"))?;
                server::handle(host, player, &request)
            };
            if let Some(data) = data {
                if let Some(reply) = TO_CLIENT.get() {
                    // 发送失败（如对方断线）不是错误，忽略即可。
                    let _ = reply.reply(cx, &data);
                }
            }
            Ok(())
        });
        let to_client = view.play_to_client_typed::<ServerData>(PayloadId(DATA_ID.into()), |data, _| {
            client::receive(data);
            Ok(())
        });
        let _ = TO_SERVER.set(to_server);
        let _ = TO_CLIENT.set(to_client);
    }

    fn client_mining_predict(
        &self,
        _host: &dyn ClientHost,
        event: &mut mod_api::ClientMiningPredictEvent,
    ) -> mod_api::Flow {
        client::record_mining(&event.prediction);
        mod_api::Flow::Continue
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPostEvent) {
        if let Some(request) = client::tick(host) {
            if let Some(token) = TO_SERVER.get() {
                // 服务端没装本 mod 时返回 UnsupportedPeer，忽略。
                let _ = token.send(host, &request);
            }
        }
    }

    fn player_logged_in(&self, _host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        server::remember_name(event.player, event.name);
    }

    fn player_logged_out(&self, _host: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        server::forget(event.player);
    }

    fn gui_render_post(&self, host: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if event.gui_hidden || host.hud_mode() != HudMode::AllVisible || host.observed_screen_id().is_some() {
            return;
        }
        if let Some(tip) = client::current() {
            render::draw(event.batch, &tip);
        }
    }
}
