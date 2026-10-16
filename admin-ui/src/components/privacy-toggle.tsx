import { Eye, EyeOff } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { setPrivacyMask, usePrivacyMask } from '@/lib/privacy'

/** 顶栏按钮：切换账号邮箱等标识的打码。默认打码，截图前不用记得去开。 */
export function PrivacyToggle() {
  const masked = usePrivacyMask()
  return (
    <Button
      variant="ghost"
      size="icon"
      onClick={() => setPrivacyMask(!masked)}
      title={masked ? '已打码账号邮箱（点击显示明文）' : '正在显示明文邮箱（点击打码）'}
      aria-label={masked ? '显示明文' : '隐藏敏感信息'}
      aria-pressed={masked}
    >
      {masked ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
    </Button>
  )
}
