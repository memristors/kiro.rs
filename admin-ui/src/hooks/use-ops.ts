import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  diagnoseProxy,
  getAutoProxy,
  getBackups,
  getCreditHistory,
  getLiveRequests,
  getQueueConfig,
  runBackup,
  setAutoProxy,
  setQueueConfig,
  type DiagnoseTarget,
  type QueueConfigUpdate,
  type SetAutoProxyRequest,
} from '@/api/ops'

export function useCreditHistory(credentialId: number | null, days: number, enabled = true) {
  return useQuery({
    queryKey: ['stats', 'credit-history', credentialId ?? 'all', days],
    queryFn: () => getCreditHistory(credentialId, days),
    enabled,
    staleTime: 25_000,
    placeholderData: keepPreviousData,
    refetchOnWindowFocus: false,
  })
}

export function useBackups() {
  return useQuery({
    queryKey: ['backups'],
    queryFn: getBackups,
    staleTime: 10_000,
    refetchOnWindowFocus: false,
    retry: false,
  })
}

export function useRunBackup() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: runBackup,
    onSuccess: (result) => {
      queryClient.setQueryData(['backups'], result.status)
    },
  })
}

export function useAutoProxy() {
  return useQuery({
    queryKey: ['auto-proxy'],
    queryFn: getAutoProxy,
    staleTime: 10_000,
    refetchOnWindowFocus: false,
  })
}

export function useSetAutoProxy() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: SetAutoProxyRequest) => setAutoProxy(body),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['auto-proxy'] }),
  })
}

/** 排队 / 并发 / 渐进冷却配置；每 3 秒刷新一次，设置页上的「当前排队」才是活的 */
export function useQueueConfig() {
  return useQuery({
    queryKey: ['queue-config'],
    queryFn: getQueueConfig,
    refetchInterval: 3000,
    staleTime: 1000,
    refetchOnWindowFocus: false,
  })
}

export function useSetQueueConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: QueueConfigUpdate) => setQueueConfig(body),
    onSuccess: (data) => queryClient.setQueryData(['queue-config'], data),
  })
}

/** 进行中的请求，每秒刷新 */
export function useLiveRequests() {
  return useQuery({
    queryKey: ['traces', 'live'],
    queryFn: getLiveRequests,
    refetchInterval: 1000,
    staleTime: 500,
    refetchOnWindowFocus: false,
    placeholderData: keepPreviousData,
  })
}

export function useDiagnoseProxy() {
  return useMutation({
    mutationFn: (target: DiagnoseTarget) => diagnoseProxy(target),
  })
}
