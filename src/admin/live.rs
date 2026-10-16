//! 实时请求登记：正在进行的请求处于哪个阶段（排队 / 连接中 / 等首字 / 输出中）。
//!
//! 请求日志（traces 表）只在请求结束时写一行，进行中的请求在页面上是看不到的，
//! 排队等待时尤其让人分不清「是卡住了还是在排队」。这里在内存里登记每个进行中的请求，
//! 阶段随请求推进更新，请求结束（成功 / 失败 / 中断）时移除，页面轮询 `GET /traces/live` 展示。
//!
//! 阶段：
//! - `queued`：没有账号立即可用（冷却中 / 并发已满 / RPM 用尽），在等待队列里
//! - `connecting`：已选好账号，请求已发往上游，等待响应头（含重试）
//! - `generating`：上游响应头已到，等待第一个内容
//! - `streaming`：内容正在输出
//!
//! 请求结束后的状态（成功 / 失败 / 中断）仍由 traces 表给出。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use parking_lot::Mutex;
use serde::Serialize;

use crate::kiro::queue::QueueHook;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Queued,
    Connecting,
    Generating,
    Streaming,
}

struct State {
    stage: Stage,
    credential_id: u64,
    attempt: u32,
    queued_since: Option<Instant>,
    queued_total_ms: u64,
    first_token_ms: Option<u64>,
}

/// 一个进行中的请求
pub struct LiveEntry {
    trace_id: String,
    ts: String,
    model: String,
    key_id: u64,
    is_stream: bool,
    client_ip: Option<String>,
    started_at: Instant,
    state: Mutex<State>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<LiveEntry>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<LiveEntry>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

impl LiveEntry {
    /// 登记一个新请求（初始阶段为 `connecting`，选号需要排队时会切到 `queued`）。
    /// 必须在请求结束时调用 [`Self::finish`] 移除。
    pub fn register(
        trace_id: String,
        ts: String,
        model: String,
        key_id: u64,
        is_stream: bool,
        client_ip: Option<String>,
    ) -> Arc<Self> {
        let entry = Arc::new(Self {
            trace_id: trace_id.clone(),
            ts,
            model,
            key_id,
            is_stream,
            client_ip,
            started_at: Instant::now(),
            state: Mutex::new(State {
                stage: Stage::Connecting,
                credential_id: 0,
                attempt: 0,
                queued_since: None,
                queued_total_ms: 0,
                first_token_ms: None,
            }),
        });
        registry().lock().insert(trace_id, Arc::clone(&entry));
        entry
    }

    pub fn set_queued(&self, queued: bool) {
        let mut state = self.state.lock();
        if queued {
            if state.queued_since.is_none() {
                state.queued_since = Some(Instant::now());
            }
            state.stage = Stage::Queued;
        } else if let Some(since) = state.queued_since.take() {
            state.queued_total_ms += since.elapsed().as_millis() as u64;
            if state.stage == Stage::Queued {
                state.stage = Stage::Connecting;
            }
        }
    }

    /// 选中了某个账号，开始一次上游尝试
    pub fn set_selected(&self, credential_id: u64) {
        let mut state = self.state.lock();
        state.credential_id = credential_id;
        state.attempt += 1;
        if state.stage != Stage::Queued {
            state.stage = Stage::Connecting;
        }
    }

    /// 上游响应头已到
    pub fn set_headers_received(&self) {
        let mut state = self.state.lock();
        if matches!(state.stage, Stage::Queued | Stage::Connecting) {
            state.stage = Stage::Generating;
        }
    }

    /// 第一个内容已输出
    pub fn set_first_token(&self) {
        let mut state = self.state.lock();
        if state.first_token_ms.is_none() {
            state.first_token_ms = Some(self.started_at.elapsed().as_millis() as u64);
        }
        state.stage = Stage::Streaming;
    }

    /// 本请求累计排队等待的毫秒数（含仍在排队的那一段）
    pub fn queued_ms(&self) -> u64 {
        let state = self.state.lock();
        state.queued_total_ms
            + state
                .queued_since
                .map(|since| since.elapsed().as_millis() as u64)
                .unwrap_or(0)
    }

    /// 请求结束：从登记表移除，返回累计排队毫秒数
    pub fn finish(&self) -> u64 {
        registry().lock().remove(&self.trace_id);
        self.queued_ms()
    }

    pub fn stage(&self) -> Stage {
        self.state.lock().stage
    }

    fn snapshot(&self) -> LiveSnapshot {
        let state = self.state.lock();
        let queued_ms = state.queued_total_ms
            + state
                .queued_since
                .map(|since| since.elapsed().as_millis() as u64)
                .unwrap_or(0);
        LiveSnapshot {
            trace_id: self.trace_id.clone(),
            ts: self.ts.clone(),
            model: self.model.clone(),
            key_id: self.key_id,
            is_stream: self.is_stream,
            client_ip: self.client_ip.clone(),
            stage: state.stage,
            credential_id: state.credential_id,
            attempt: state.attempt,
            elapsed_ms: self.started_at.elapsed().as_millis() as u64,
            queued_ms,
            first_token_ms: state.first_token_ms,
        }
    }
}

impl QueueHook for LiveEntry {
    fn queued(&self, queued: bool) {
        self.set_queued(queued);
    }
}

/// 进行中请求的快照（`GET /traces/live`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSnapshot {
    pub trace_id: String,
    /// 请求开始时间（RFC3339）
    pub ts: String,
    pub model: String,
    pub key_id: u64,
    pub is_stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
    pub stage: Stage,
    /// 当前（或最近一次）选中的账号；0 = 还没选到
    pub credential_id: u64,
    /// 已发起的上游尝试次数（>1 表示重试 / 切换过账号）
    pub attempt: u32,
    pub elapsed_ms: u64,
    /// 累计排队等待毫秒数
    pub queued_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
}

/// 当前所有进行中的请求，按开始时间从早到晚
pub fn snapshot() -> Vec<LiveSnapshot> {
    let mut out: Vec<LiveSnapshot> = registry().lock().values().map(|e| e.snapshot()).collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Arc<LiveEntry> {
        LiveEntry::register(
            id.to_string(),
            chrono::Utc::now().to_rfc3339(),
            "claude-opus-5.5".to_string(),
            0,
            true,
            None,
        )
    }

    #[test]
    fn stages_advance_through_a_normal_request() {
        let e = entry("live-normal");
        assert_eq!(e.stage(), Stage::Connecting);
        e.set_selected(3);
        assert_eq!(e.stage(), Stage::Connecting);
        e.set_headers_received();
        assert_eq!(e.stage(), Stage::Generating);
        e.set_first_token();
        assert_eq!(e.stage(), Stage::Streaming);
        let snap = snapshot().into_iter().find(|s| s.trace_id == "live-normal").unwrap();
        assert_eq!(snap.credential_id, 3);
        assert_eq!(snap.attempt, 1);
        assert!(snap.first_token_ms.is_some());
        e.finish();
        assert!(snapshot().iter().all(|s| s.trace_id != "live-normal"));
    }

    #[test]
    fn queueing_is_visible_and_accounted() {
        let e = entry("live-queued");
        e.set_queued(true);
        assert_eq!(e.stage(), Stage::Queued);
        std::thread::sleep(std::time::Duration::from_millis(30));
        // 还在排队时就能看到已等待的时间
        assert!(e.queued_ms() >= 25);
        // 排队中选号不会把阶段改回 connecting
        e.set_selected(1);
        assert_eq!(e.stage(), Stage::Queued);
        e.set_queued(false);
        assert_eq!(e.stage(), Stage::Connecting);
        let total = e.finish();
        assert!(total >= 25);
    }

    #[test]
    fn retries_count_attempts_and_return_to_connecting() {
        let e = entry("live-retry");
        e.set_selected(1);
        e.set_selected(2);
        let snap = snapshot().into_iter().find(|s| s.trace_id == "live-retry").unwrap();
        assert_eq!(snap.attempt, 2);
        assert_eq!(snap.credential_id, 2);
        e.finish();
    }

    #[test]
    fn stage_never_moves_backwards_after_headers() {
        let e = entry("live-order");
        e.set_first_token();
        e.set_headers_received();
        assert_eq!(e.stage(), Stage::Streaming, "晚到的响应头通知不能把输出中退回等首字");
        e.finish();
    }
}
