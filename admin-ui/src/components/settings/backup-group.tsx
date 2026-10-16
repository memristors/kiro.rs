import { toast } from 'sonner'
import { DatabaseBackup } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { SettingGroup, SettingRow } from '@/components/console/setting-row'
import { useBackups, useRunBackup } from '@/hooks/use-ops'
import { extractErrorMessage } from '@/lib/utils'

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  return `${(bytes / 1024).toFixed(1)} KB`
}

function formatTime(value?: string): string {
  return value ? new Date(value).toLocaleString('zh-CN') : '—'
}

/**
 * 凭据备份：只读展示 + 「立即备份」。
 *
 * 开关、间隔、保留份数写在 config.json（credentialBackupEnabled / credentialBackupIntervalMinutes /
 * credentialBackupKeep），改动需要重启，所以这里不提供编辑。还原故意不做成一键按钮：
 * 凭据文件由进程持有并会写回，还原要停服务、替换文件，误点的代价比手动操作大得多。
 */
export function BackupGroup() {
  const { data, isLoading, error } = useBackups()
  const run = useRunBackup()

  const onRun = () =>
    run.mutate(undefined, {
      onSuccess: (result) =>
        toast.success(result.created ? `已备份：${result.created}` : '凭据文件没有变化，未生成新备份'),
      onError: (e) => toast.error('备份失败：' + extractErrorMessage(e)),
    })

  return (
    <SettingGroup
      title="凭据备份"
      description="定时复制 credentials.json 到数据目录下的 backups/，凭据文件被写坏或误删时用来找回。内容没变化时不会重复备份。"
    >
      {isLoading && <div className="py-3 text-sm text-muted-foreground">加载中…</div>}
      {error && !isLoading && (
        <div className="py-3 text-sm text-muted-foreground">备份功能不可用：{extractErrorMessage(error)}</div>
      )}
      {data && (
        <>
          <SettingRow
            label="策略"
            hint="在 config.json 里调整 credentialBackupEnabled / credentialBackupIntervalMinutes / credentialBackupKeep，重启后生效"
          >
            <span className="console-num text-[13px]">
              {data.enabled
                ? `每 ${data.intervalMinutes} 分钟 · 保留最新 ${data.keep} 份`
                : '定时备份已关闭（仍可手动备份）'}
            </span>
          </SettingRow>
          <SettingRow
            label="最近一次"
            hint={
              data.lastError ? (
                <span className="text-red-600 dark:text-red-400">失败：{data.lastError}</span>
              ) : data.nextRunAt ? (
                `下次约 ${formatTime(data.nextRunAt)}`
              ) : undefined
            }
          >
            <div className="flex items-center gap-2">
              <span className="console-num text-[13px]">{formatTime(data.lastRunAt)}</span>
              <Button size="sm" variant="outline" onClick={onRun} disabled={run.isPending}>
                <DatabaseBackup className="h-3.5 w-3.5" />
                {run.isPending ? '备份中…' : '立即备份'}
              </Button>
            </div>
          </SettingRow>
          <SettingRow label="备份文件" hint={`目录：${data.dir}`}>
            {data.backups.length === 0 ? (
              <span className="text-[13px] text-muted-foreground">还没有备份</span>
            ) : (
              <ul className="space-y-0.5 text-[12.5px]">
                {data.backups.slice(0, 8).map((b) => (
                  <li key={b.name} className="console-num flex items-center gap-3">
                    <span className="font-mono">{b.name}</span>
                    <span className="text-muted-foreground">{formatSize(b.size)}</span>
                  </li>
                ))}
                {data.backups.length > 8 && (
                  <li className="text-muted-foreground">另有 {data.backups.length - 8} 份更早的</li>
                )}
              </ul>
            )}
          </SettingRow>
          <SettingRow
            label="如何还原"
            hint="停服务 → 用备份覆盖 credentials.json → 启动服务。不提供一键还原，避免误点覆盖当前账号。"
          >
            <code className="rounded bg-muted px-2 py-1 text-[12px]">
              systemctl stop kiro-rs &amp;&amp; cp backups/&lt;文件&gt; credentials.json &amp;&amp; systemctl start kiro-rs
            </code>
          </SettingRow>
        </>
      )}
    </SettingGroup>
  )
}
