import { useEffect, useState } from 'react'
import { Activity, Hourglass } from 'lucide-react'
import type { LiveRequest, LiveStage } from '@/api/ops'
import { useLiveRequests } from '@/hooks/use-ops'
import { cn } from '@/lib/utils'

/** 阶段 → 文案、说明和颜色。「排队」「处理」「输出」分得越清楚，越容易判断请求是卡住了还是在等。 */
export const STAGES: Record<LiveStage, { label: string; hint: string; className: string; pulse: boolean }> = {
  queued: {
    label: '排队中',
    hint: '没有账号立即可用（冷却中 / 并发已满），在等待队列里',
    className: 'bg-amber-500/15 text-amber-700 dark:text-amber-300 border-amber-500/40',
    pulse: true,
  },
  connecting: {
    label: '连接上游',
    hint: '已选好账号，请求已发往上游，等待响应头',
    className: 'bg-sky-500/15 text-sky-700 dark:text-sky-300 border-sky-500/40',
    pulse: true,
  },
  generating: {
    label: '生成中',
    hint: '上游已接受请求，等待第一个内容（长上下文 / 推理模型这一段会比较久）',
    className: 'bg-violet-500/15 text-violet-700 dark:text-violet-300 border-violet-500/40',
    pulse: true,
  },
  streaming: {
    label: '输出中',
    hint: '内容正在输出',
    className: 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300 border-emerald-500/40',
    pulse: true,
  },
}

const STAGE_ORDER: LiveStage[] = ['queued', 'connecting', 'generating', 'streaming']

export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.floor((ms % 60_000) / 1000)
  return `${minutes}m${String(seconds).padStart(2, '0')}s`
}

/** 每 500ms 刷新一次的「现在」，让耗时在两次轮询之间也能走动 */
export function useNow(intervalMs = 500): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(timer)
  }, [intervalMs])
  return now
}

/**
 * 请求日志页顶部的「进行中」面板：正在处理的请求逐条显示阶段、账号、已用时间。
 * 请求结束后消失，进入下方的历史列表（成功 / 失败 / 中断）。
 */
export function LiveRequestsPanel({ compact = false }: { compact?: boolean }) {
  const { data, dataUpdatedAt } = useLiveRequests()
  const now = useNow()
  const requests = data?.requests ?? []
  // 服务端给的是「取数那一刻」的耗时；加上取数之后过去的时间，数字才是连续走动的
  const drift = dataUpdatedAt ? Math.max(0, now - dataUpdatedAt) : 0

  const counts: Record<LiveStage, number> = { queued: 0, connecting: 0, generating: 0, streaming: 0 }
  for (const r of requests) counts[r.stage] += 1

  return (
    <div className="rounded-lg border border-border/60 bg-card">
      <div
        className={cn(
          'flex flex-wrap items-center gap-x-4 gap-y-1 px-3 py-2',
          !compact && 'border-b border-border/50',
        )}
      >
        <span className="inline-flex items-center gap-1.5 text-sm font-medium">
          <Activity className="h-4 w-4 text-primary" />
          进行中
          <span className="console-num rounded bg-muted px-1.5 text-xs">{requests.length}</span>
        </span>
        {STAGE_ORDER.map((stage) => (
          <span
            key={stage}
            title={STAGES[stage].hint}
            className={cn(
              'inline-flex items-center gap-1 text-xs',
              counts[stage] === 0 && 'text-muted-foreground/60',
            )}
          >
            {STAGES[stage].label}
            <span className="console-num font-medium">{counts[stage]}</span>
          </span>
        ))}
        {compact && requests.length > 0 && (
          <span className="text-[11px] text-muted-foreground">明细见下方列表最上面几行</span>
        )}
        {data && data.accountMaxConcurrency > 0 && (
          <span className="ml-auto text-[11px] text-muted-foreground">
            单账号并发上限 {data.accountMaxConcurrency}
          </span>
        )}
      </div>

      {compact ? null : requests.length === 0 ? (
        <div className="px-3 py-3 text-xs text-muted-foreground">当前没有进行中的请求</div>
      ) : (
        <ul className="divide-y divide-border/40">
          {requests.map((r) => (
            <LiveRow key={r.traceId} request={r} drift={drift} />
          ))}
        </ul>
      )}
    </div>
  )
}

function LiveRow({ request: r, drift }: { request: LiveRequest; drift: number }) {
  const stage = STAGES[r.stage]
  const elapsed = r.elapsedMs + drift
  // 仍在排队时，排队时间也要继续走
  const queued = r.queuedMs + (r.stage === 'queued' ? drift : 0)
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-xs">
      <span
        title={stage.hint}
        className={cn(
          'inline-flex w-[4.5rem] items-center justify-center gap-1 rounded border px-1.5 py-0.5 font-medium',
          stage.className,
        )}
      >
        {r.stage === 'queued' ? (
          <Hourglass className="h-3 w-3" />
        ) : (
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-current opacity-60" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-current" />
          </span>
        )}
        {stage.label}
      </span>
      <span className="console-num font-mono">{r.model}</span>
      <span className="text-muted-foreground">
        {r.credentialId > 0 ? `账号 #${r.credentialId}` : '尚未选号'}
      </span>
      {r.attempt > 1 && (
        <span
          className="rounded bg-amber-500/15 px-1.5 text-amber-700 dark:text-amber-300"
          title="之前的尝试失败了，已重试或换了账号"
        >
          第 {r.attempt} 次尝试
        </span>
      )}
      {r.keyId !== 0 && <span className="text-muted-foreground">Key #{r.keyId}</span>}
      <span className="ml-auto flex items-center gap-3 console-num">
        {queued >= 500 && (
          <span className="text-amber-700 dark:text-amber-300" title="累计排队等待时间">
            排队 {formatMs(queued)}
          </span>
        )}
        {r.firstTokenMs != null && (
          <span className="text-muted-foreground" title="首个内容到达用时">
            首字 {formatMs(r.firstTokenMs)}
          </span>
        )}
        <span className="font-medium tabular-nums" title="从收到请求起的总用时">
          {formatMs(elapsed)}
        </span>
      </span>
    </li>
  )
}
