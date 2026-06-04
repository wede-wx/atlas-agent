//! P1-2: run 暂停 / 恢复信号。
//!
//! 暂停是**可逆**的,所以不能复用一次性的 `CancellationToken`。每个活跃 run
//! 持有一个 [`RunPauseHandle`]:命令层置位 `paused`,agent 主循环在工具边界
//! (发起下一次模型调用之前)调用 [`RunPauseHandle::wait_until_resumed`] 真正
//! 挂起,期间不调模型、不烧 CPU,`messages` 上下文留在调用栈上,resume 后从断
//! 点续跑——而不是重开(见 P1-2 假完成红线)。
//!
//! 暂停期间的**取消**由命令层 `select!` 从外部 abort 整个 future 处理,与本模块
//! 无关:无论循环 await 在哪一点,future 被 drop 即停止。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

/// 单个 run 的暂停状态 + 恢复唤醒。
#[derive(Debug, Default)]
pub struct RunPauseHandle {
    paused: AtomicBool,
    resume: Notify,
}

impl RunPauseHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// 置为暂停。返回 `true` 表示状态由 running 翻转为 paused(用于避免重复落库 / 发事件)。
    pub fn pause(&self) -> bool {
        !self.paused.swap(true, Ordering::SeqCst)
    }

    /// 解除暂停并唤醒在安全点等待的循环。返回 `true` 表示状态由 paused 翻转为 running。
    pub fn resume(&self) -> bool {
        let was_paused = self.paused.swap(false, Ordering::SeqCst);
        if was_paused {
            self.resume.notify_waiters();
        }
        was_paused
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// 在 agent 主循环的安全点调用:若处于暂停态则一直 await 到 resume。
    ///
    /// 先注册 `notified()` 再二次检查 `is_paused`,避免 pause→resume 恰好发生在
    /// 注册间隙时丢失唤醒(经典 lost-wakeup 防护)。即便某次唤醒被错过,下一轮
    /// `while` 也会读到 `paused == false` 而退出,所以不会永久挂起。
    pub async fn wait_until_resumed(&self) {
        while self.is_paused() {
            let notified = self.resume.notified();
            if !self.is_paused() {
                break;
            }
            notified.await;
        }
    }
}

/// 按 `run_id` 索引活跃 run 的暂停句柄。命令层插入 / 移除,agent 循环只读取。
pub type RunPauseRegistry = Arc<Mutex<HashMap<String, Arc<RunPauseHandle>>>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_then_resume_toggles_state_and_reports_transitions() {
        let handle = RunPauseHandle::new();
        assert!(!handle.is_paused());
        assert!(handle.pause(), "running -> paused 应报告状态翻转");
        assert!(handle.is_paused());
        assert!(!handle.pause(), "已 paused 再 pause 不应再报告翻转");
        assert!(handle.resume(), "paused -> running 应报告状态翻转");
        assert!(!handle.is_paused());
        assert!(!handle.resume(), "已 running 再 resume 不应报告翻转");
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_not_paused() {
        let handle = RunPauseHandle::new();
        handle.wait_until_resumed().await; // 未暂停:不应阻塞
    }

    #[tokio::test]
    async fn wait_blocks_until_resumed() {
        let handle = Arc::new(RunPauseHandle::new());
        handle.pause();
        let waiter = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.wait_until_resumed().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "暂停中的 waiter 不应提前完成");
        handle.resume();
        waiter.await.expect("resume 后 waiter 应完成");
    }
}
