//! Helpers every area's tests share: a headless session on the demo puppet and the three MCP calls.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "integration test helpers")]

use std::time::{Duration, Instant};

use anim_mcp::{Backend, Confinement, HeadlessSession, HostInfo, Request, spawn_headless};
use charcraft_app::hooks::{Hooks, make_host};
use charcraft_app::live::LiveLink;
use charcraft_app::session::{Links, registry};
use charcraft_app::world::World;
use serde_json::{Value, json};

/// A headless session on the demo puppet; `setup` may change the hooks (for example to swap in a mock MIDI device).
pub fn headless_with(setup: impl FnOnce(&mut Hooks) + Send + 'static) -> HeadlessSession {
    spawn_headless(move || {
        let opened = World::demo().unwrap();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        let mut host =
            make_host(opened, links, registry().unwrap(), Confinement::unrestricted(), HostInfo { name: "charcraft".into(), version: "test".into() });
        setup(host.hooks_mut());
        host
    })
    .unwrap()
}

/// A headless session on the demo puppet.
pub fn headless() -> HeadlessSession {
    headless_with(|_| {})
}

/// Runs a command and returns its result.
pub fn run(backend: &Backend, id: &str, params: Value) -> Result<Value, String> {
    backend.call(Request::CommandRun { id: id.into(), params, merge_key: None }).map(|r| r.value["result"].clone()).map_err(|e| e.to_string())
}

/// Asks a query.
pub fn query(backend: &Backend, name: &str) -> Value {
    query_with(backend, name, json!({}))
}

/// Asks a query with parameters.
pub fn query_with(backend: &Backend, name: &str, params: Value) -> Value {
    backend.call(Request::Query { name: name.into(), params }).unwrap().value
}

/// Polls `read` until `done` accepts what it returns (the host works between requests), or fails after a few seconds.
pub fn wait_for(what: &str, mut read: impl FnMut() -> Value, done: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let value = read();
        if done(&value) {
            return value;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}; last saw {value}");
        std::thread::sleep(Duration::from_millis(15));
    }
}
