import { useEffect } from 'react'
import { CheckCircle2, RefreshCw, XCircle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { DiagnoseTarget, IpInfo, SiteResult } from '@/api/ops'
import { useDiagnoseProxy } from '@/hooks/use-ops'
import { cn, extractErrorMessage } from '@/lib/utils'

/** 站点标识 → 展示名。前两项是 Kiro 自己的链路，不通就意味着账号用不了 */
const SITE_LABELS: Record<string, { name: string; hint: string }> = {
  kiroApi: { name: 'Kiro 接口', hint: 'q.us-east-1.amazonaws.com，转发请求走这里' },
  kiroAuth: { name: 'Kiro 登录刷新', hint: 'prod.us-east-1.auth.desktop.kiro.dev，刷新 token 走这里' },
  kiroSite: { name: 'kiro.dev', hint: '官网' },
  github: { name: 'GitHub', hint: '在线更新、检查版本' },
  google: { name: 'Google', hint: '通用外网连通性参照' },
  amazon: { name: 'Amazon', hint: '通用外网连通性参照' },
}

function latencyClass(site: SiteResult): string {
  if (!site.ok) return 'text-red-600 dark:text-red-400'
  if (site.latencyMs < 400) return 'text-emerald-600 dark:text-emerald-400'
  if (site.latencyMs < 1200) return 'text-amber-600 dark:text-amber-400'
  return 'text-red-600 dark:text-red-400'
}

function formatLocation(ip: IpInfo): string {
  const place = [ip.city, ip.region, ip.country ?? ip.countryCode].filter(Boolean)
  // 去掉相邻重复（城市和地区同名时）
  const unique = place.filter((v, i) => v !== place[i - 1])
  return unique.join(' · ') || '未知位置'
}

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 要测哪个代理；空对象表示全局代理（未配置时为直连） */
  target: DiagnoseTarget
  /** 对话框标题里的补充说明，比如代理地址或凭据 ID */
  subject?: string
}

/**
 * 代理网络诊断：出口 IP 的归属地和运营商，加上 Kiro 链路与常用站点的连通、延迟。
 * 打开即自动测一次，可手动重测。测试目标由服务端固定，这里只决定走哪个代理。
 */
export function ProxyDiagnoseDialog({ open, onOpenChange, target, subject }: Props) {
  const { mutate, data, error, isPending, reset } = useDiagnoseProxy()
  const targetKey = JSON.stringify(target)

  useEffect(() => {
    if (open) {
      mutate(target)
    } else {
      reset()
    }
    // target 以内容比较，避免父组件每次渲染新建对象导致重复请求
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, targetKey])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>网络诊断{subject ? ` · ${subject}` : ''}</DialogTitle>
        </DialogHeader>

        {isPending && (
          <div className="flex items-center justify-center gap-2 py-8 text-sm text-muted-foreground">
            <RefreshCw className="h-4 w-4 animate-spin" />
            正在测试，最长约 10 秒…
          </div>
        )}

        {error && !isPending && (
          <div className="space-y-2 py-4 text-center">
            <div className="flex items-center justify-center gap-2 text-red-500">
              <XCircle className="h-5 w-5" />
              <span className="font-medium">诊断失败</span>
            </div>
            <div className="px-4 text-sm text-muted-foreground">{extractErrorMessage(error)}</div>
          </div>
        )}

        {data && !isPending && (
          <div className="space-y-4">
            <div className="rounded-md border p-3 text-sm">
              <div className="mb-1 flex items-center justify-between gap-2">
                <span className="text-muted-foreground">测试出口</span>
                <span className="console-num truncate font-mono text-xs">{data.via}</span>
              </div>
              {data.ip ? (
                <div className="space-y-0.5">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">出口 IP</span>
                    <span className="console-num font-mono font-medium">{data.ip.ip}</span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">位置</span>
                    <span>{formatLocation(data.ip)}</span>
                  </div>
                  {data.ip.org && (
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">运营商</span>
                      <span className="truncate text-right">{data.ip.org}</span>
                    </div>
                  )}
                  <div className="pt-1 text-right text-[11px] text-muted-foreground">
                    数据来源 {data.ip.source}
                  </div>
                </div>
              ) : (
                <div className="text-sm text-amber-600 dark:text-amber-400">
                  {data.ipError ?? '未能查询出口 IP'}
                </div>
              )}
            </div>

            <div className="divide-y rounded-md border">
              {data.sites.map((site) => {
                const meta = SITE_LABELS[site.id] ?? { name: site.id, hint: site.url }
                return (
                  <div key={site.id} className="flex items-center gap-3 px-3 py-2 text-sm" title={meta.hint}>
                    {site.ok ? (
                      <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" />
                    ) : (
                      <XCircle className="h-4 w-4 shrink-0 text-red-500" />
                    )}
                    <div className="min-w-0 flex-1">
                      <div className="font-medium">{meta.name}</div>
                      {site.error && (
                        <div className="truncate text-xs text-red-600 dark:text-red-400" title={site.error}>
                          {site.error}
                        </div>
                      )}
                    </div>
                    {site.ok && site.status !== undefined && (
                      <span className="console-num text-xs text-muted-foreground">HTTP {site.status}</span>
                    )}
                    <span className={cn('console-num w-16 text-right tabular-nums', latencyClass(site))}>
                      {site.latencyMs} ms
                    </span>
                  </div>
                )
              })}
            </div>

            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span>
                「通」只表示收到了 HTTP 响应（含 403 等），不代表账号可用。总耗时 {data.totalMs} ms
              </span>
              <Button size="sm" variant="outline" onClick={() => mutate(target)} className="h-7 text-xs">
                <RefreshCw className="mr-1 h-3 w-3" />
                重新测试
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
