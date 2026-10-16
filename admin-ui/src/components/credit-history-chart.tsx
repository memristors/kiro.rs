import { useMemo, useState } from 'react'
import { Download } from 'lucide-react'
import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { downloadCreditHistoryCsv } from '@/api/ops'
import { useCreditHistory } from '@/hooks/use-ops'
import { cn, extractErrorMessage } from '@/lib/utils'
import {
  tooltipContentStyle,
  tooltipItemStyle,
  tooltipLabelStyle,
} from '@/components/charts/tooltip-style'

const RANGES = [
  { days: 1, label: '24 小时' },
  { days: 7, label: '7 天' },
  { days: 30, label: '30 天' },
  { days: 90, label: '90 天' },
] as const

function money(value: number): string {
  return value.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

function axisTime(ts: number, days: number): string {
  const d = new Date(ts * 1000)
  const md = `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
  if (days <= 1) return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  return md
}

/**
 * 单个凭据的额度趋势：已用额度随时间的曲线、总额参考线、区间消耗汇总，可导出 CSV。
 *
 * 数据只在额度变化时落点（外加每 6 小时一个保活点），所以折线用阶梯形表示「这段时间内额度没动」。
 */
export function CreditHistoryChart({ credentialId }: { credentialId: number }) {
  const [days, setDays] = useState<number>(7)
  const [exporting, setExporting] = useState(false)
  const { data, isLoading, error } = useCreditHistory(credentialId, days)

  const points = useMemo(
    () => (data?.points ?? []).map((p) => ({ ts: p.tsEpoch, used: p.used, limit: p.limit, delta: p.delta })),
    [data],
  )
  const summary = data?.summary[0]
  const limit = points.length ? points[points.length - 1].limit : 0
  const hasReset = (summary?.resets ?? 0) > 0

  const exportCsv = async () => {
    setExporting(true)
    try {
      await downloadCreditHistoryCsv(credentialId, days)
    } catch (e) {
      toast.error('导出失败：' + extractErrorMessage(e))
    } finally {
      setExporting(false)
    }
  }

  return (
    <div className="space-y-3 border-t pt-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">额度趋势</span>
        <div className="flex items-center gap-1">
          {RANGES.map((r) => (
            <button
              key={r.days}
              type="button"
              onClick={() => setDays(r.days)}
              className={cn(
                'rounded-md px-2 py-1 text-xs transition-colors',
                days === r.days
                  ? 'bg-primary/10 font-semibold text-primary'
                  : 'text-muted-foreground hover:bg-accent hover:text-foreground',
              )}
            >
              {r.label}
            </button>
          ))}
          <Button
            variant="ghost"
            size="sm"
            onClick={exportCsv}
            disabled={exporting || points.length === 0}
            title="导出当前时间范围的明细为 CSV"
          >
            <Download className="h-3.5 w-3.5" />
            CSV
          </Button>
        </div>
      </div>

      {isLoading && <div className="py-8 text-center text-sm text-muted-foreground">加载中…</div>}

      {error && (
        <div className="py-6 text-center text-sm text-red-500">
          读取失败：{extractErrorMessage(error)}
        </div>
      )}

      {!isLoading && !error && points.length === 0 && (
        <div className="py-8 text-center text-sm text-muted-foreground">
          这段时间内还没有记录。额度发生变化时才会落点，刚上线的话过一会儿再看。
        </div>
      )}

      {points.length > 0 && (
        <>
          <div className="h-48 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={points} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="currentColor" strokeOpacity={0.1} />
                <XAxis
                  dataKey="ts"
                  type="number"
                  scale="time"
                  domain={['dataMin', 'dataMax']}
                  tickFormatter={(v: number) => axisTime(v, days)}
                  tick={{ fontSize: 11 }}
                  minTickGap={32}
                />
                <YAxis
                  domain={[0, (max: number) => Math.max(max, limit) * 1.05]}
                  tickFormatter={(v: number) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : String(Math.round(v)))}
                  tick={{ fontSize: 11 }}
                  width={44}
                />
                <Tooltip
                  contentStyle={tooltipContentStyle}
                  labelStyle={tooltipLabelStyle}
                  itemStyle={tooltipItemStyle}
                  labelFormatter={(v) => new Date(Number(v) * 1000).toLocaleString('zh-CN')}
                  formatter={(value) => [`$${money(Number(value))}`, '已用']}
                />
                {limit > 0 && (
                  <ReferenceLine
                    y={limit}
                    stroke="#ef4444"
                    strokeDasharray="4 4"
                    label={{ value: `额度 $${money(limit)}`, position: 'insideTopRight', fontSize: 11, fill: '#ef4444' }}
                  />
                )}
                <Line
                  type="stepAfter"
                  dataKey="used"
                  stroke="#3b82f6"
                  strokeWidth={2}
                  dot={points.length <= 60}
                  isAnimationActive={false}
                />
              </LineChart>
            </ResponsiveContainer>
          </div>

          {summary && (
            <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm sm:grid-cols-4">
              <Stat label="区间消耗" value={`$${money(summary.burned)}`} />
              <Stat label="日均消耗" value={`$${money(summary.burnedPerDay)}`} />
              <Stat
                label="额度重置"
                value={`${summary.resets} 次`}
                hint={hasReset ? '已用额度回落，说明经历了计费周期重置' : undefined}
              />
              <Stat label="数据点" value={String(summary.points)} />
            </div>
          )}
          {data?.truncated && (
            <div className="text-xs text-amber-600 dark:text-amber-400">
              数据点过多，只显示了最早的一部分。缩短时间范围可以看到完整明细。
            </div>
          )}
        </>
      )}
    </div>
  )
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div title={hint}>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="font-medium tabular-nums">{value}</div>
    </div>
  )
}
