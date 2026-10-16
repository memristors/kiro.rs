import { useSetQueueConfig, useQueueConfig } from '@/hooks/use-ops'
import {
  SettingGroup,
  SettingNumber,
  SettingReadout,
  SettingSwitch,
  useFieldSaver,
} from '@/components/console/setting-row'
import { reportSaveError } from '@/components/settings/report-error'
import type { QueueConfigUpdate } from '@/api/ops'

/**
 * 排队与并发：避免一个账号被限流后，流量全压到其余账号上，把它们也打进冷却（雪崩）。
 *
 * - 渐进冷却：被限流的账号先只歇「首次冷却」这么久，连续被限流才翻倍加长，最长到「账号冷却时长」；
 *   成功一次就清零。
 * - 排队：暂时没有账号可用（冷却中 / 并发已满）时，请求在服务里等最近一个账号恢复，而不是立刻失败；
 *   等不到才返回 429，并带 Retry-After 让客户端按时间退避。
 * - 单账号并发上限：同一账号同时处理的请求数有上限，超出的排队，避免突发把账号打爆。
 * 所有改动即时生效。
 */
export function QueueGroup() {
  const { data, isLoading } = useQueueConfig()
  const { mutate } = useSetQueueConfig()
  const saver = useFieldSaver<QueueConfigUpdate>(mutate, reportSaveError)

  const inFlightEntries = Object.entries(data?.inFlight ?? {})
  const totalInFlight = inFlightEntries.reduce((sum, [, n]) => sum + n, 0)
  const queueEnabled = data?.queueEnabled ?? true

  return (
    <SettingGroup
      title="排队与并发"
      description="账号被限流时不让流量压垮其余账号：渐进冷却、请求内排队等待、单账号并发上限。"
    >
      <SettingNumber
        label="首次冷却"
        hint={
          data
            ? `账号第一次被限流只歇这么久；连续被限流会翻倍加长，最长 ${data.cooldownMaxSecs} 秒（见上方「账号冷却时长」），成功一次就清零`
            : '账号第一次被限流只歇这么久'
        }
        value={data?.cooldownBaseSecs ?? 20}
        onCommit={(secs) => saver.save('cooldownBase', { cooldownBaseSecs: secs })}
        min={1}
        max={Math.max(1, data?.cooldownMaxSecs ?? 3600)}
        unit="秒"
        presets={[10, 20, 30, 60]}
        pending={saver.isSaving('cooldownBase')}
        saved={saver.isSaved('cooldownBase')}
        disabled={isLoading}
      />
      <SettingSwitch
        label="请求排队等待"
        hint={
          queueEnabled
            ? '没有账号立即可用时，请求在服务里等最近一个账号恢复，而不是立刻失败'
            : '没有账号可用就直接返回 429（带 Retry-After），不排队'
        }
        checked={queueEnabled}
        onChange={(next) => saver.save('queueEnabled', { queueEnabled: next })}
        pending={saver.isSaving('queueEnabled')}
        saved={saver.isSaved('queueEnabled')}
        disabled={isLoading}
      />
      <SettingNumber
        label="最长排队等待"
        hint="排队超过这么久还没有账号可用，就返回 429 并带 Retry-After"
        value={data?.queueMaxWaitSecs ?? 30}
        onCommit={(secs) => saver.save('maxWait', { queueMaxWaitSecs: secs })}
        min={1}
        max={600}
        unit="秒"
        presets={[10, 30, 60, 120]}
        pending={saver.isSaving('maxWait')}
        saved={saver.isSaved('maxWait')}
        disabled={isLoading || !queueEnabled}
      />
      <SettingNumber
        label="排队人数上限"
        hint="同时排队的请求数上限；队列满了新请求直接 429，避免积压无限增长"
        value={data?.queueMaxWaiters ?? 64}
        onCommit={(n) => saver.save('maxWaiters', { queueMaxWaiters: n })}
        min={1}
        max={10000}
        unit="个"
        presets={[16, 64, 256]}
        pending={saver.isSaving('maxWaiters')}
        saved={saver.isSaved('maxWaiters')}
        disabled={isLoading || !queueEnabled}
      />
      <SettingNumber
        label="单账号并发上限"
        hint="同一账号同时处理的请求数上限，超出的排队；0 表示不限制。流式回答会一直占着名额直到输出结束"
        value={data?.accountMaxConcurrency ?? 0}
        onCommit={(n) => saver.save('maxConcurrency', { accountMaxConcurrency: n })}
        min={0}
        max={100}
        unit="个"
        presets={[0, 2, 3, 5]}
        pending={saver.isSaving('maxConcurrency')}
        saved={saver.isSaved('maxConcurrency')}
        disabled={isLoading}
      />
      <SettingReadout
        label="当前状态"
        hint="每 3 秒刷新；请求日志页有逐条请求的实时进度"
      >
        排队 {data?.waiting ?? 0} · 处理中 {totalInFlight}
        {inFlightEntries.length > 0 && (
          <span className="ml-2">
            ({inFlightEntries.map(([id, n]) => `#${id}×${n}`).join(' ')})
          </span>
        )}
      </SettingReadout>
    </SettingGroup>
  )
}
