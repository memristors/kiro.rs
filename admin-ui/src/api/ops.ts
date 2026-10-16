import axios from 'axios'
import { storage } from '@/lib/storage'

/**
 * 运维类接口：额度历史、凭据备份、代理网络诊断。
 * 与 stats.ts 一样自带 axios 实例，鉴权头由请求拦截器补上。
 */
const api = axios.create({
  baseURL: '/api/admin',
  timeout: 30000,
  headers: { 'Content-Type': 'application/json' },
})

api.interceptors.request.use((config) => {
  const apiKey = storage.getApiKey()
  if (apiKey) config.headers['x-api-key'] = apiKey
  return config
})

// ---------- 额度历史 ----------

export interface CreditPoint {
  credentialId: number
  tsEpoch: number
  used: number
  limit: number
  delta: number
}

export interface CreditSummary {
  credentialId: number
  email?: string
  points: number
  firstAt: string
  lastAt: string
  firstUsed: number
  lastUsed: number
  limit: number
  burned: number
  burnedPerDay: number
  resets: number
}

export interface CreditHistoryResponse {
  days: number
  retentionDays: number
  truncated: boolean
  points: CreditPoint[]
  summary: CreditSummary[]
}

export async function getCreditHistory(
  credentialId: number | null,
  days: number,
): Promise<CreditHistoryResponse> {
  const { data } = await api.get<CreditHistoryResponse>('/stats/credit-history', {
    params: { days, ...(credentialId !== null ? { credentialId } : {}) },
  })
  return data
}

/** 下载 CSV：接口要求带鉴权头，不能直接用 <a href>，所以先取成 Blob 再触发保存 */
export async function downloadCreditHistoryCsv(
  credentialId: number | null,
  days: number,
): Promise<void> {
  const resp = await api.get<Blob>('/stats/credit-history', {
    params: { days, format: 'csv', ...(credentialId !== null ? { credentialId } : {}) },
    responseType: 'blob',
  })
  const name = `credit-history${credentialId !== null ? `-${credentialId}` : ''}-${days}d.csv`
  const url = URL.createObjectURL(resp.data)
  const link = document.createElement('a')
  link.href = url
  link.download = name
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}

// ---------- 凭据备份 ----------

export interface BackupInfo {
  name: string
  size: number
  modifiedAt: string
}

export interface BackupStatus {
  enabled: boolean
  intervalMinutes: number
  keep: number
  dir: string
  lastRunAt?: string
  lastError?: string
  nextRunAt?: string
  backups: BackupInfo[]
}

export async function getBackups(): Promise<BackupStatus> {
  const { data } = await api.get<BackupStatus>('/backups')
  return data
}

export async function runBackup(): Promise<{ created: string | null; status: BackupStatus }> {
  const { data } = await api.post<{ created: string | null; status: BackupStatus }>('/backups/run')
  return data
}

// ---------- 代理网络诊断 ----------

export interface IpInfo {
  ip: string
  country?: string
  countryCode?: string
  region?: string
  city?: string
  org?: string
  timezone?: string
  source: string
}

export interface SiteResult {
  id: string
  url: string
  ok: boolean
  status?: number
  latencyMs: number
  error?: string
}

export interface NetworkCheckResult {
  via: string
  ip?: IpInfo
  ipError?: string
  sites: SiteResult[]
  totalMs: number
}

/** 三选一指定目标；全不填则测全局代理（未配置时测直连） */
export interface DiagnoseTarget {
  proxyId?: number
  credentialId?: number
  proxyUrl?: string
  proxyUsername?: string
  proxyPassword?: string
}

// ---------- 新凭据自动代理 ----------

export interface AutoProxyConfig {
  enabled: boolean
  proxyUrl: string | null
  /** `{id}` 会被替换成新凭据的 ID */
  usernameTemplate: string | null
  passwordSet: boolean
}

export interface SetAutoProxyRequest {
  enabled: boolean
  proxyUrl?: string | null
  usernameTemplate?: string | null
  /** 省略表示保留现有密码，空字符串表示清除 */
  password?: string
}

export async function getAutoProxy(): Promise<AutoProxyConfig> {
  const { data } = await api.get<AutoProxyConfig>('/config/auto-proxy')
  return data
}

export async function setAutoProxy(body: SetAutoProxyRequest): Promise<void> {
  await api.put('/config/auto-proxy', body)
}

export async function diagnoseProxy(target: DiagnoseTarget): Promise<NetworkCheckResult> {
  // 单次诊断最长约 8s（IP）+ 10s（站点）并行，留足余量
  const { data } = await api.post<NetworkCheckResult>('/proxy-pool/diagnose', target, {
    timeout: 45000,
  })
  return data
}
