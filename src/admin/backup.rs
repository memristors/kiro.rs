//! 凭据文件定时备份
//!
//! `credentials.json` 里放着所有账号的 refreshToken，被写坏或误删后没有别处可以找回。
//! 这里按固定间隔把它复制到 `<数据目录>/backups/`，只保留最新的若干份。
//!
//! 备份只做复制，不改动原文件，也不参与凭据读写路径：备份失败只记日志和状态，不影响服务。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;

const FILE_PREFIX: &str = "credentials-";
const FILE_SUFFIX: &str = ".json";

/// 备份间隔下限（分钟）：每份备份是整个凭据文件，间隔太短只会白写磁盘
pub const MIN_INTERVAL_MINUTES: u32 = 5;
/// 保留份数上限：防止误填成几万份把磁盘写满
pub const MAX_KEEP: u32 = 500;
/// 启动后到首次备份的等待时间，让凭据文件先完成启动时的整理写回
const FIRST_RUN_DELAY: Duration = Duration::from_secs(20);

/// 单个备份文件的信息
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub name: String,
    pub size: u64,
    pub modified_at: String,
}

/// 备份状态（Admin 页面展示用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub keep: u32,
    pub dir: String,
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    pub next_run_at: Option<String>,
    /// 新的在前
    pub backups: Vec<BackupInfo>,
}

#[derive(Default)]
struct RunState {
    last_run_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

/// 凭据备份管理器
pub struct BackupManager {
    source: PathBuf,
    dir: PathBuf,
    enabled: bool,
    interval_minutes: u32,
    keep: u32,
    state: Mutex<RunState>,
}

pub type SharedBackupManager = Arc<BackupManager>;

impl BackupManager {
    pub fn new(
        source: PathBuf,
        dir: PathBuf,
        enabled: bool,
        interval_minutes: u32,
        keep: u32,
    ) -> Self {
        Self {
            source,
            dir,
            enabled,
            interval_minutes: interval_minutes.max(MIN_INTERVAL_MINUTES),
            keep: keep.clamp(1, MAX_KEEP),
            state: Mutex::new(RunState::default()),
        }
    }

    /// 立即备份一次。内容与最新一份完全相同时跳过。
    ///
    /// 返回新备份的文件名；因内容未变化而跳过时返回 `None`。
    pub fn run_once(&self) -> anyhow::Result<Option<String>> {
        let result = self.run_once_inner();
        let mut state = self.state.lock();
        state.last_run_at = Some(Utc::now());
        state.last_error = result.as_ref().err().map(|e| format!("{:#}", e));
        result
    }

    fn run_once_inner(&self) -> anyhow::Result<Option<String>> {
        let data = std::fs::read(&self.source)
            .with_context(|| format!("读取凭据文件失败: {}", self.source.display()))?;
        // 不是合法 JSON（写到一半、被截断）就不备份，避免用坏数据把好备份挤出保留窗口
        serde_json::from_slice::<serde_json::Value>(&data)
            .context("凭据文件不是合法 JSON，已跳过本次备份")?;

        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("创建备份目录失败: {}", self.dir.display()))?;
        restrict_dir(&self.dir);

        if let Some(latest) = self.list_files()?.last() {
            if std::fs::read(self.dir.join(&latest.name)).ok().as_deref() == Some(data.as_slice())
            {
                return Ok(None);
            }
        }

        let name = format!(
            "{}{}{}",
            FILE_PREFIX,
            Utc::now().format("%Y%m%d-%H%M%S"),
            FILE_SUFFIX
        );
        let path = self.dir.join(&name);
        let tmp = self.dir.join(format!("{}.tmp", name));
        std::fs::write(&tmp, &data).context("写入备份文件失败")?;
        restrict_file(&tmp);
        std::fs::rename(&tmp, &path).context("备份文件改名失败")?;

        self.prune()?;
        Ok(Some(name))
    }

    /// 备份目录里符合命名的文件，按文件名（即时间）升序
    fn list_files(&self) -> anyhow::Result<Vec<BackupInfo>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e).context("读取备份目录失败"),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(FILE_PREFIX) || !name.ends_with(FILE_SUFFIX) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let modified_at = meta
                .modified()
                .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
                .unwrap_or_default();
            out.push(BackupInfo {
                name,
                size: meta.len(),
                modified_at,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// 只保留最新 `keep` 份
    fn prune(&self) -> anyhow::Result<()> {
        let files = self.list_files()?;
        let excess = files.len().saturating_sub(self.keep as usize);
        for file in files.into_iter().take(excess) {
            if let Err(e) = std::fs::remove_file(self.dir.join(&file.name)) {
                tracing::warn!("删除旧凭据备份 {} 失败: {}", file.name, e);
            }
        }
        Ok(())
    }

    pub fn status(&self) -> BackupStatus {
        let mut backups = self.list_files().unwrap_or_default();
        backups.reverse();
        let state = self.state.lock();
        BackupStatus {
            enabled: self.enabled,
            interval_minutes: self.interval_minutes,
            keep: self.keep,
            dir: self.dir.display().to_string(),
            last_run_at: state.last_run_at.map(|t| t.to_rfc3339()),
            last_error: state.last_error.clone(),
            next_run_at: if self.enabled {
                state
                    .last_run_at
                    .map(|t| (t + chrono::Duration::minutes(self.interval_minutes as i64)).to_rfc3339())
            } else {
                None
            },
            backups,
        }
    }

    /// 启动后台定时备份。关闭时只记一行日志。
    pub fn spawn(self: &Arc<Self>) {
        if !self.enabled {
            tracing::info!("凭据定时备份已关闭");
            return;
        }
        tracing::info!(
            "凭据定时备份已开启：每 {} 分钟一次，保留最新 {} 份，目录 {}",
            self.interval_minutes,
            self.keep,
            self.dir.display()
        );
        let this = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_RUN_DELAY).await;
            loop {
                let worker = Arc::clone(&this);
                match tokio::task::spawn_blocking(move || worker.run_once()).await {
                    Ok(Ok(Some(name))) => tracing::info!("凭据已备份: {}", name),
                    Ok(Ok(None)) => tracing::debug!("凭据文件无变化，跳过备份"),
                    Ok(Err(e)) => tracing::warn!("凭据备份失败: {:#}", e),
                    Err(e) => tracing::warn!("凭据备份任务异常: {}", e),
                }
                tokio::time::sleep(Duration::from_secs(this.interval_minutes as u64 * 60)).await;
            }
        });
    }
}

/// 备份里是 refreshToken，目录仅属主可访问（非 Unix 平台无操作）
#[cfg(unix)]
fn restrict_dir(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &std::path::Path) {}

#[cfg(unix)]
fn restrict_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_file(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiro-backup-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manager(root: &PathBuf, keep: u32) -> BackupManager {
        BackupManager::new(
            root.join("credentials.json"),
            root.join("backups"),
            true,
            30,
            keep,
        )
    }

    #[test]
    fn creates_backup_and_skips_identical_content() {
        let root = temp_dir("skip");
        std::fs::write(root.join("credentials.json"), r#"[{"id":1}]"#).unwrap();
        let m = manager(&root, 3);

        assert!(m.run_once().unwrap().is_some());
        assert!(m.run_once().unwrap().is_none(), "内容未变应跳过");
        assert_eq!(m.status().backups.len(), 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn invalid_json_is_not_backed_up_and_error_is_recorded() {
        let root = temp_dir("badjson");
        std::fs::write(root.join("credentials.json"), r#"[{"id":"#).unwrap();
        let m = manager(&root, 3);

        assert!(m.run_once().is_err());
        let status = m.status();
        assert!(status.backups.is_empty());
        assert!(status.last_error.is_some());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn prune_keeps_only_newest_files() {
        let root = temp_dir("prune");
        let m = manager(&root, 2);
        std::fs::create_dir_all(root.join("backups")).unwrap();
        for stamp in ["20260101-000001", "20260101-000002", "20260101-000003", "20260101-000004"] {
            std::fs::write(
                root.join("backups").join(format!("credentials-{}.json", stamp)),
                "[]",
            )
            .unwrap();
        }
        // 无关文件不应被清理
        std::fs::write(root.join("backups").join("notes.txt"), "keep me").unwrap();

        m.prune().unwrap();

        let names: Vec<String> = m.status().backups.into_iter().map(|b| b.name).collect();
        assert_eq!(
            names,
            vec![
                "credentials-20260101-000004.json".to_string(),
                "credentials-20260101-000003.json".to_string()
            ]
        );
        assert!(root.join("backups").join("notes.txt").exists());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn interval_and_keep_are_clamped() {
        let m = BackupManager::new(PathBuf::from("a"), PathBuf::from("b"), true, 1, 100_000);
        let status = m.status();
        assert_eq!(status.interval_minutes, MIN_INTERVAL_MINUTES);
        assert_eq!(status.keep, MAX_KEEP);
    }
}
