import { useSyncExternalStore } from 'react'

/**
 * 隐私打码：截图、录屏、投屏时不暴露账号邮箱等标识。
 *
 * 默认开启，只有用户在顶栏显式关闭才显示明文。打码结果由原文哈希决定：同一个邮箱
 * 永远得到同一个打码串，所以截图里仍能分辨「这是不是同一个账号」，又看不出是谁。
 */

const STORAGE_KEY = 'privacyMask'
const CHANGE_EVENT = 'privacy-mask-change'

export function getPrivacyMask(): boolean {
  if (typeof localStorage === 'undefined') return true
  try {
    return localStorage.getItem(STORAGE_KEY) !== 'off'
  } catch {
    return true
  }
}

export function setPrivacyMask(on: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, on ? 'on' : 'off')
  } catch {
    // 存储不可用（隐私模式等）时，本次会话内仍然生效
  }
  window.dispatchEvent(new Event(CHANGE_EVENT))
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, onChange)
  window.addEventListener('storage', onChange)
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange)
    window.removeEventListener('storage', onChange)
  }
}

/** 组件内读取打码开关；开关变化会触发重新渲染 */
export function usePrivacyMask(): boolean {
  return useSyncExternalStore(subscribe, getPrivacyMask, () => true)
}

/** cyrb53 字符串哈希：稳定、分布均匀，不需要加密强度 */
function hash(input: string): number {
  let h1 = 0xdeadbeef
  let h2 = 0x41c6ce57
  for (let i = 0; i < input.length; i++) {
    const ch = input.charCodeAt(i)
    h1 = Math.imul(h1 ^ ch, 2654435761)
    h2 = Math.imul(h2 ^ ch, 1597334677)
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909)
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909)
  return 4294967296 * (2097151 & h2) + (h1 >>> 0)
}

function shortHash(input: string): string {
  return hash(input.trim().toLowerCase()).toString(16).padStart(8, '0').slice(-4)
}

/** `alice@example.com` → `a***3f9c@e***.com`；没有 @ 的文本按整体处理 */
export function maskEmail(email: string): string {
  const value = email.trim()
  if (!value) return value
  const at = value.lastIndexOf('@')
  if (at <= 0) return `${value[0]}***${shortHash(value)}`
  const local = value.slice(0, at)
  const domain = value.slice(at + 1)
  const dot = domain.lastIndexOf('.')
  const tld = dot > 0 ? domain.slice(dot) : ''
  return `${local[0]}***${shortHash(value)}@${domain[0] ?? ''}***${tld}`
}

/** 密钥类文本只露出头尾：`sk-kiro-abcdef…1234` */
export function maskSecret(secret: string): string {
  const value = secret.trim()
  if (value.length <= 10) return '••••••'
  return `${value.slice(0, 6)}••••${value.slice(-4)}`
}

/** 非组件场景（工具函数、模板字符串）用：按当前开关返回展示用的邮箱。调用它的组件需另外调用 usePrivacyMask() 以便开关变化时重新渲染。 */
export function displayEmail(value?: string | null): string {
  if (!value) return ''
  return getPrivacyMask() ? maskEmail(value) : value
}

/**
 * 组件里统一取打码函数：
 * `const mask = useMasker(); <span>{mask.email(c.email)}</span>`
 */
export function useMasker() {
  const on = usePrivacyMask()
  return {
    on,
    email: (value?: string | null): string => (value ? (on ? maskEmail(value) : value) : ''),
    secret: (value?: string | null): string => (value ? (on ? maskSecret(value) : value) : ''),
  }
}
