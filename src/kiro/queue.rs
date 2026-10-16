//! 调度闸门：账号并发上限、请求内排队等待、有界队列，以及账号限流的渐进冷却。
//!
//! 背景：账号被上游限流（429）后，旧逻辑固定冷却 300 秒，并把流量全部转给其余账号。
//! 账号不多时，其余账号随即也被限流，全部进入冷却，之后每个请求都在 0 ms 内失败，
//! 客户端又立刻重试——一个账号的限流放大成整个池子的雪崩。
//!
//! 这里的做法：
//! 1. **渐进冷却**：第一次限流只冷却很短（默认 20 秒），连续被限流才翻倍加长，直到上限；成功一次就清零。
//! 2. **每账号并发上限**：同一账号同时在处理的请求数有上限，超出的排队，避免突发把账号打爆。
//! 3. **请求内排队等待**：没有账号可用但只是暂时的（冷却中 / RPM 用尽 / 并发已满）时，请求挂起等最近一个
//!    账号恢复，最多等 `queue_max_wait_secs` 秒，而不是立刻失败。
//! 4. **有界队列**：同时等待的请求数有上限；等不到或队列满时返回 429 并带 `Retry-After`，让客户端退避。
//!
//! 本模块只含与具体凭据结构无关的部分（计数、等待、决策），决策函数是纯函数，便于单元测试。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// 请求进出等待队列的观察者（实时请求日志用它显示「排队」状态）。
///
/// 选号发生在很深的调用栈里，拿不到请求级对象；provider 在调用选号前用 task-local 把观察者
/// 挂到当前任务上，这里通过 [`notify_queue`] 通知，没挂观察者时是空操作。
pub trait QueueHook: Send + Sync {
    fn queued(&self, queued: bool);
}

tokio::task_local! {
    pub static QUEUE_HOOK: Arc<dyn QueueHook>;
}

/// 通知当前任务的观察者：进入（true）或离开（false）等待队列
pub fn notify_queue(queued: bool) {
    let _ = QUEUE_HOOK.try_with(|hook| hook.queued(queued));
}

/// 等待时的最长单次睡眠。到期或被唤醒后都会重新评估，所以这只是兜底轮询间隔。
const MAX_POLL: Duration = Duration::from_secs(1);
/// 连续限流计数的遗忘时间：距上次限流超过这么久，再被限流按"第一次"算。
pub const STREAK_FORGET_SECS: u64 = 600;

pub const DEFAULT_COOLDOWN_BASE_SECS: u64 = 20;
pub const DEFAULT_QUEUE_MAX_WAIT_SECS: u64 = 30;
pub const DEFAULT_QUEUE_MAX_WAITERS: u32 = 64;

/// 渐进冷却时长（秒）。
///
/// - `streak` 是本次限流之后的连续次数（第一次为 1）。
/// - 上游明确给了 Retry-After 时以它为准（夹到 `1..=max`）；否则 `base × 2^(streak-1)`，不超过 `max`。
pub fn next_cooldown_secs(base: u64, max: u64, streak: u32, explicit: Option<u64>) -> u64 {
    let base = base.max(1);
    let max = max.max(base);
    if let Some(retry_after) = explicit {
        return retry_after.clamp(1, max);
    }
    let shift = streak.saturating_sub(1).min(20);
    base.saturating_mul(1u64 << shift).min(max)
}

/// 某个候选账号对"当前请求"的可用情况
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// 预计多久后可用；`None` 表示要等某个在途请求结束才知道（并发已满）
    pub ready_in: Option<Duration>,
}

/// 没有立即可用的账号时的处理决定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 没有任何"只是暂时不可用"的账号（全部被禁用或不匹配），走原来的错误路径
    NoCandidate,
    /// 继续等待，最长睡这么久后重新评估
    Wait(Duration),
    /// 不再等：返回 429，`Retry-After` 取这个秒数
    Reject { retry_after_secs: u64 },
}

pub struct DecideInput<'a> {
    pub queue_enabled: bool,
    /// 本请求已经等了多久
    pub waited: Duration,
    pub max_wait: Duration,
    /// 等待队列是否已满（对已经在队列里的请求不适用）
    pub queue_full: bool,
    pub already_waiting: bool,
    pub candidates: &'a [Candidate],
}

fn ceil_secs(duration: Duration) -> u64 {
    (duration.as_secs() + u64::from(duration.subsec_nanos() > 0)).max(1)
}

pub fn decide(input: &DecideInput<'_>) -> Decision {
    if input.candidates.is_empty() {
        return Decision::NoCandidate;
    }
    let earliest_known = input.candidates.iter().filter_map(|c| c.ready_in).min();
    let has_unknown = input.candidates.iter().any(|c| c.ready_in.is_none());
    // 并发已满的账号没有确切时间：给客户端一个保守的短退避
    let retry_after_secs = earliest_known.map(ceil_secs).unwrap_or(2);
    let reject = Decision::Reject { retry_after_secs };

    if !input.queue_enabled {
        return reject;
    }
    let budget = input.max_wait.saturating_sub(input.waited);
    if budget.is_zero() {
        return reject;
    }
    if !input.already_waiting && input.queue_full {
        return reject;
    }
    match (earliest_known, has_unknown) {
        // 全部是已知的冷却时间：等不起就直接拒绝，别白占队列
        (Some(ready), false) if ready > budget => reject,
        (Some(ready), _) => Decision::Wait(ready.min(budget).clamp(Duration::from_millis(10), MAX_POLL)),
        // 只有并发已满的账号：等在途请求结束的通知
        (None, _) => Decision::Wait(budget.clamp(Duration::from_millis(10), MAX_POLL)),
    }
}

/// 可在设置页调整的参数（读写时整体传递）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueSettings {
    pub cooldown_base_secs: u64,
    pub queue_enabled: bool,
    pub queue_max_wait_secs: u64,
    pub queue_max_waiters: u32,
    pub account_max_concurrency: u32,
}

impl QueueSettings {
    /// 校验取值范围；`cooldown_max_secs` 是账号限流冷却的上限，首次冷却不能超过它
    pub fn validate(&self, cooldown_max_secs: u64) -> anyhow::Result<()> {
        if !(1..=3600).contains(&self.cooldown_base_secs) {
            anyhow::bail!("首次冷却秒数必须在 1..=3600 内: {}", self.cooldown_base_secs);
        }
        if self.cooldown_base_secs > cooldown_max_secs {
            anyhow::bail!(
                "首次冷却（{} 秒）不能超过冷却上限（{} 秒）",
                self.cooldown_base_secs,
                cooldown_max_secs
            );
        }
        if !(1..=600).contains(&self.queue_max_wait_secs) {
            anyhow::bail!("最长排队等待秒数必须在 1..=600 内: {}", self.queue_max_wait_secs);
        }
        if !(1..=10_000).contains(&self.queue_max_waiters) {
            anyhow::bail!("排队人数上限必须在 1..=10000 内: {}", self.queue_max_waiters);
        }
        if self.account_max_concurrency > 100 {
            anyhow::bail!(
                "单账号并发上限必须在 0..=100 内（0 表示不限）: {}",
                self.account_max_concurrency
            );
        }
        Ok(())
    }
}

/// 在途请求计数 + 等待队列 + 运行时可调参数
pub struct DispatchGate {
    pub queue_enabled: AtomicBool,
    pub queue_max_wait_secs: AtomicU64,
    pub queue_max_waiters: AtomicU32,
    /// 每账号并发上限；0 表示不限
    pub max_concurrency: AtomicU32,
    pub cooldown_base_secs: AtomicU64,
    counts: Mutex<HashMap<u64, u32>>,
    waiting: AtomicUsize,
    notify: Notify,
}

impl DispatchGate {
    pub fn new(
        queue_enabled: bool,
        queue_max_wait_secs: u64,
        queue_max_waiters: u32,
        max_concurrency: u32,
        cooldown_base_secs: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            queue_enabled: AtomicBool::new(queue_enabled),
            queue_max_wait_secs: AtomicU64::new(queue_max_wait_secs),
            queue_max_waiters: AtomicU32::new(queue_max_waiters),
            max_concurrency: AtomicU32::new(max_concurrency),
            cooldown_base_secs: AtomicU64::new(cooldown_base_secs.max(1)),
            counts: Mutex::new(HashMap::new()),
            waiting: AtomicUsize::new(0),
            notify: Notify::new(),
        })
    }

    pub fn settings(&self) -> QueueSettings {
        QueueSettings {
            cooldown_base_secs: self.cooldown_base_secs.load(Ordering::Relaxed),
            queue_enabled: self.queue_enabled.load(Ordering::Relaxed),
            queue_max_wait_secs: self.queue_max_wait_secs.load(Ordering::Relaxed),
            queue_max_waiters: self.queue_max_waiters.load(Ordering::Relaxed),
            account_max_concurrency: self.max_concurrency.load(Ordering::Relaxed),
        }
    }

    pub fn apply(&self, settings: QueueSettings) {
        self.cooldown_base_secs
            .store(settings.cooldown_base_secs.max(1), Ordering::Relaxed);
        self.queue_enabled
            .store(settings.queue_enabled, Ordering::Relaxed);
        self.queue_max_wait_secs
            .store(settings.queue_max_wait_secs, Ordering::Relaxed);
        self.queue_max_waiters
            .store(settings.queue_max_waiters, Ordering::Relaxed);
        self.max_concurrency
            .store(settings.account_max_concurrency, Ordering::Relaxed);
        // 上限放宽 / 队列参数变化后，让等待者立刻重新评估
        self.wake();
    }

    /// 账号当前在途请求数
    pub fn in_flight(&self, id: u64) -> u32 {
        self.counts.lock().get(&id).copied().unwrap_or(0)
    }

    pub fn snapshot(&self) -> HashMap<u64, u32> {
        self.counts.lock().iter().filter(|(_, n)| **n > 0).map(|(k, v)| (*k, *v)).collect()
    }

    /// 并发上限已满（上限为 0 时恒为 false）
    pub fn at_cap(&self, id: u64) -> bool {
        let cap = self.max_concurrency.load(Ordering::Relaxed);
        cap > 0 && self.in_flight(id) >= cap
    }

    /// 占一个并发名额。上限已满返回 `None`（检查和占用在同一把锁内，避免并发请求同时越过上限）。
    /// 上限为 0 时也会计数，用于展示在途请求。
    pub fn try_acquire(self: &Arc<Self>, id: u64) -> Option<FlightGuard> {
        let cap = self.max_concurrency.load(Ordering::Relaxed);
        let mut counts = self.counts.lock();
        let count = counts.entry(id).or_insert(0);
        if cap > 0 && *count >= cap {
            return None;
        }
        *count += 1;
        Some(FlightGuard {
            gate: Arc::clone(self),
            id,
        })
    }

    fn release(&self, id: u64) {
        {
            let mut counts = self.counts.lock();
            if let Some(count) = counts.get_mut(&id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&id);
                }
            }
        }
        self.notify.notify_waiters();
    }

    /// 通知等待者重新评估（冷却被解除、凭据启用、配置变化等）
    pub fn wake(&self) {
        self.notify.notify_waiters();
    }

    /// 当前排队等待的请求数
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::Relaxed)
    }

    pub fn queue_full(&self) -> bool {
        self.waiting() >= self.queue_max_waiters.load(Ordering::Relaxed) as usize
    }

    /// 进入等待队列；队列已满返回 `None`。返回的守卫离开作用域（含客户端断开导致 future 被丢弃）时自动退出队列。
    pub fn enter_queue(self: &Arc<Self>) -> Option<WaiterGuard> {
        let max = self.queue_max_waiters.load(Ordering::Relaxed) as usize;
        let previous = self.waiting.fetch_add(1, Ordering::Relaxed);
        if previous >= max {
            self.waiting.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        notify_queue(true);
        Some(WaiterGuard {
            gate: Arc::clone(self),
        })
    }

    /// 睡到 `timeout` 或被唤醒
    pub async fn sleep_or_wake(&self, timeout: Duration) {
        let _ = tokio::time::timeout(timeout, self.notify.notified()).await;
    }
}

/// 在途请求名额。随请求（含流式响应体）一起存活，释放时自动归还并唤醒等待者。
pub struct FlightGuard {
    gate: Arc<DispatchGate>,
    id: u64,
}

impl FlightGuard {
    pub fn credential_id(&self) -> u64 {
        self.id
    }
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        self.gate.release(self.id);
    }
}

/// 等待队列中的一个位置
pub struct WaiterGuard {
    gate: Arc<DispatchGate>,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.gate.waiting.fetch_sub(1, Ordering::Relaxed);
        notify_queue(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn cooldown_grows_geometrically_and_caps() {
        let seq: Vec<u64> = (1..=8).map(|s| next_cooldown_secs(20, 300, s, None)).collect();
        assert_eq!(seq, vec![20, 40, 80, 160, 300, 300, 300, 300]);
    }

    #[test]
    fn cooldown_honors_explicit_retry_after_within_bounds() {
        assert_eq!(next_cooldown_secs(20, 300, 5, Some(7)), 7);
        assert_eq!(next_cooldown_secs(20, 300, 1, Some(9999)), 300);
        assert_eq!(next_cooldown_secs(20, 300, 1, Some(0)), 1);
    }

    #[test]
    fn cooldown_max_never_below_base() {
        assert_eq!(next_cooldown_secs(60, 10, 1, None), 60);
        assert_eq!(next_cooldown_secs(0, 0, 1, None), 1);
        // 极大的连续次数不会溢出
        assert_eq!(next_cooldown_secs(20, u64::MAX, 200, None), 20 * (1u64 << 20));
    }

    fn input<'a>(candidates: &'a [Candidate]) -> DecideInput<'a> {
        DecideInput {
            queue_enabled: true,
            waited: Duration::ZERO,
            max_wait: secs(30),
            queue_full: false,
            already_waiting: false,
            candidates,
        }
    }

    #[test]
    fn no_candidates_defers_to_old_error_path() {
        assert_eq!(decide(&input(&[])), Decision::NoCandidate);
    }

    #[test]
    fn waits_when_recovery_fits_in_budget() {
        let c = [Candidate { ready_in: Some(secs(12)) }];
        // 睡眠被限制在轮询间隔内，到期后重新评估
        assert_eq!(decide(&input(&c)), Decision::Wait(MAX_POLL));
        let c = [Candidate { ready_in: Some(Duration::from_millis(300)) }];
        assert_eq!(decide(&input(&c)), Decision::Wait(Duration::from_millis(300)));
    }

    #[test]
    fn rejects_immediately_when_recovery_is_beyond_the_budget() {
        let c = [Candidate { ready_in: Some(secs(120)) }, Candidate { ready_in: Some(secs(200)) }];
        assert_eq!(decide(&input(&c)), Decision::Reject { retry_after_secs: 120 });
    }

    #[test]
    fn earliest_candidate_decides() {
        let c = [Candidate { ready_in: Some(secs(500)) }, Candidate { ready_in: Some(secs(5)) }];
        assert!(matches!(decide(&input(&c)), Decision::Wait(_)));
    }

    #[test]
    fn concurrency_saturation_waits_for_a_release() {
        let c = [Candidate { ready_in: None }];
        assert_eq!(decide(&input(&c)), Decision::Wait(MAX_POLL));
        // 一个在冷却很久、一个并发已满：仍然值得等并发那个
        let c = [Candidate { ready_in: Some(secs(500)) }, Candidate { ready_in: None }];
        assert!(matches!(decide(&input(&c)), Decision::Wait(_)));
    }

    #[test]
    fn budget_exhaustion_rejects_with_retry_after() {
        let c = [Candidate { ready_in: Some(secs(3)) }];
        let mut i = input(&c);
        i.waited = secs(30);
        assert_eq!(decide(&i), Decision::Reject { retry_after_secs: 3 });
        let c = [Candidate { ready_in: None }];
        let mut i = input(&c);
        i.waited = secs(31);
        assert_eq!(decide(&i), Decision::Reject { retry_after_secs: 2 });
    }

    #[test]
    fn full_queue_rejects_newcomers_but_not_those_already_waiting() {
        let c = [Candidate { ready_in: Some(secs(5)) }];
        let mut i = input(&c);
        i.queue_full = true;
        assert_eq!(decide(&i), Decision::Reject { retry_after_secs: 5 });
        i.already_waiting = true;
        assert!(matches!(decide(&i), Decision::Wait(_)));
    }

    #[test]
    fn disabled_queue_rejects_with_retry_after_instead_of_waiting() {
        let c = [Candidate { ready_in: Some(secs(5)) }];
        let mut i = input(&c);
        i.queue_enabled = false;
        assert_eq!(decide(&i), Decision::Reject { retry_after_secs: 5 });
    }

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        let c = [Candidate { ready_in: Some(Duration::from_millis(40_300)) }];
        let mut i = input(&c);
        i.max_wait = secs(5);
        assert_eq!(decide(&i), Decision::Reject { retry_after_secs: 41 });
    }

    #[test]
    fn concurrency_cap_is_enforced_and_released_by_drop() {
        let gate = DispatchGate::new(true, 30, 64, 2, 20);
        let a = gate.try_acquire(7).expect("first slot");
        let _b = gate.try_acquire(7).expect("second slot");
        assert!(gate.at_cap(7));
        assert!(gate.try_acquire(7).is_none(), "第三个应被拒绝");
        // 其它账号不受影响
        assert!(gate.try_acquire(8).is_some());
        drop(a);
        assert!(!gate.at_cap(7));
        assert!(gate.try_acquire(7).is_some());
    }

    #[test]
    fn zero_cap_means_unlimited_but_still_counts() {
        let gate = DispatchGate::new(true, 30, 64, 0, 20);
        let guards: Vec<_> = (0..50).map(|_| gate.try_acquire(1).unwrap()).collect();
        assert!(!gate.at_cap(1));
        assert_eq!(gate.in_flight(1), 50);
        assert_eq!(gate.snapshot().get(&1), Some(&50));
        drop(guards);
        assert_eq!(gate.in_flight(1), 0);
        assert!(gate.snapshot().is_empty());
    }

    #[test]
    fn queue_is_bounded_and_slots_return_on_drop() {
        let gate = DispatchGate::new(true, 30, 2, 0, 20);
        let a = gate.enter_queue().expect("slot 1");
        let _b = gate.enter_queue().expect("slot 2");
        assert!(gate.queue_full());
        assert!(gate.enter_queue().is_none());
        assert_eq!(gate.waiting(), 2);
        drop(a);
        assert_eq!(gate.waiting(), 1);
        assert!(gate.enter_queue().is_some());
    }

    #[tokio::test]
    async fn release_wakes_a_sleeping_waiter_early() {
        let gate = DispatchGate::new(true, 30, 64, 1, 20);
        let guard = gate.try_acquire(1).unwrap();
        let waiter = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move {
                let started = std::time::Instant::now();
                gate.sleep_or_wake(Duration::from_secs(10)).await;
                started.elapsed()
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);
        let elapsed = waiter.await.unwrap();
        assert!(elapsed < Duration::from_secs(2), "应被释放事件唤醒，实际等了 {:?}", elapsed);
    }
}
