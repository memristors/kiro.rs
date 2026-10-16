import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  diagnoseProxy,
  getBackups,
  getCreditHistory,
  runBackup,
  type DiagnoseTarget,
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

export function useDiagnoseProxy() {
  return useMutation({
    mutationFn: (target: DiagnoseTarget) => diagnoseProxy(target),
  })
}
