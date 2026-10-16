import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  diagnoseProxy,
  getAutoProxy,
  getBackups,
  getCreditHistory,
  runBackup,
  setAutoProxy,
  type DiagnoseTarget,
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

export function useDiagnoseProxy() {
  return useMutation({
    mutationFn: (target: DiagnoseTarget) => diagnoseProxy(target),
  })
}
