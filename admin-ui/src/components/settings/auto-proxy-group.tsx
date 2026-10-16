import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { SettingGroup, SettingRow } from '@/components/console/setting-row'
import { useAutoProxy, useSetAutoProxy } from '@/hooks/use-ops'
import { extractErrorMessage, maskProxyUrl } from '@/lib/utils'

const PROXY_SCHEMES = ['http://', 'https://', 'socks4://', 'socks4a://', 'socks5://', 'socks5h://']

/** 模板预览：把 {id} 换成示例 ID */
function preview(template: string, id: number): string {
  return template.trim().split('{id}').join(String(id))
}

/**
 * 新凭据自动代理：手动添加（含登录、导入）的凭据如果没有自己的代理，就按这里的规则分配一个。
 *
 * 用户名模板里的 {id} 会换成凭据 ID。配合「用户名绑定出口」的代理入口使用最合适，
 * 例如 kiro-session-k{id}：每个账号固定在自己的出口，不同账号落到不同出口。
 * 只影响之后新增的凭据；已有凭据不会被改动；凭据自己填了代理（包括 direct）以它为准。
 */
export function AutoProxyGroup() {
  const { data, isLoading } = useAutoProxy()
  const save = useSetAutoProxy()

  const [enabled, setEnabled] = useState(false)
  const [url, setUrl] = useState('')
  const [template, setTemplate] = useState('')
  const [password, setPassword] = useState('')

  useEffect(() => {
    if (!data) return
    setEnabled(data.enabled)
    setUrl(data.proxyUrl ?? '')
    setTemplate(data.usernameTemplate ?? '')
    setPassword('')
  }, [data])

  const dirty =
    !!data &&
    (enabled !== data.enabled ||
      url.trim() !== (data.proxyUrl ?? '') ||
      template.trim() !== (data.usernameTemplate ?? '') ||
      password !== '')

  const onSave = () => {
    const trimmed = url.trim()
    if (enabled && !trimmed) {
      toast.error('开启自动代理需要填写代理地址')
      return
    }
    if (trimmed && !PROXY_SCHEMES.some((s) => trimmed.toLowerCase().startsWith(s))) {
      toast.error(`代理地址需以 ${PROXY_SCHEMES.join(' / ')} 开头`)
      return
    }
    save.mutate(
      {
        enabled,
        proxyUrl: trimmed || null,
        usernameTemplate: template.trim() || null,
        ...(password ? { password } : {}),
      },
      {
        onSuccess: () => toast.success('已保存，之后新增的凭据会按这个规则分配代理'),
        onError: (e) => toast.error('保存失败：' + extractErrorMessage(e)),
      },
    )
  }

  const clearPassword = () =>
    save.mutate(
      { enabled, proxyUrl: url.trim() || null, usernameTemplate: template.trim() || null, password: '' },
      {
        onSuccess: () => toast.success('已清除密码'),
        onError: (e) => toast.error('清除失败：' + extractErrorMessage(e)),
      },
    )

  const busy = isLoading || save.isPending
  const templateHasId = template.includes('{id}')

  return (
    <SettingGroup
      title="新凭据自动代理"
      description="手动添加（含登录、导入）的凭据如果没有自己的代理，就按这里的规则分配一个。只影响之后新增的凭据，已有凭据不会被改动。"
    >
      <SettingRow label="启用" hint="关闭时新凭据按原来的规则：用自己的代理，没有则走全局代理" pending={save.isPending}>
        <Switch checked={enabled} disabled={busy} onCheckedChange={setEnabled} aria-label="启用新凭据自动代理" />
      </SettingRow>
      <SettingRow
        label="代理地址"
        hint={data?.proxyUrl ? `当前：${maskProxyUrl(data.proxyUrl)}` : '例如 http://10.3.34.10:8443，支持 HTTP / HTTPS / SOCKS'}
      >
        <Input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://host:port"
          disabled={busy}
          spellCheck={false}
          autoComplete="off"
          className="console-num h-8 w-[min(20rem,60vw)] text-[12.5px]"
        />
      </SettingRow>
      <SettingRow
        label="用户名模板"
        hint={
          <>
            <code>{'{id}'}</code> 会换成新凭据的 ID。
            {template.trim() ? `例如凭据 #7 → ${preview(template, 7)}` : '留空则不带认证用户名'}
            {template.trim() && !templateHasId && (
              <span className="text-amber-600 dark:text-amber-400">
                {' '}
                模板里没有 {'{id}'}，所有新凭据会用同一个用户名，落到同一个出口
              </span>
            )}
          </>
        }
      >
        <Input
          value={template}
          onChange={(e) => setTemplate(e.target.value)}
          placeholder="kiro-session-k{id}"
          disabled={busy}
          spellCheck={false}
          autoComplete="off"
          className="console-num h-8 w-[min(20rem,60vw)] text-[12.5px]"
        />
      </SettingRow>
      <SettingRow
        label="密码"
        hint={data?.passwordSet ? '已配置密码；留空保存时保留现有密码' : '选填，代理要求认证时填写'}
      >
        <div className="flex flex-wrap items-center gap-1.5">
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={data?.passwordSet ? '已配置，留空则保留' : '留空则不认证'}
            disabled={busy}
            spellCheck={false}
            autoComplete="new-password"
            className="console-num h-8 w-[min(16rem,50vw)] text-[12.5px]"
          />
          {data?.passwordSet && (
            <Button size="sm" variant="ghost" onClick={clearPassword} disabled={busy} title="清除已保存的密码">
              清除密码
            </Button>
          )}
        </div>
      </SettingRow>
      <div className="flex justify-end pt-2">
        <Button size="sm" onClick={onSave} disabled={busy || !dirty}>
          {save.isPending ? '保存中…' : '保存'}
        </Button>
      </div>
    </SettingGroup>
  )
}
