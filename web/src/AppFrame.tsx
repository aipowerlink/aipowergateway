import { useState, useEffect, useCallback } from 'react'
import styles from './AppFrame.module.css'
import { Sidebar } from './Sidebar'
import { MemberList } from './MemberList'
import { UsageTable } from './UsageTable'
import { ControlsPanel } from './ControlsPanel'
import { DetailsPanel } from './DetailsPanel'
import type { Member, View } from './types'

// 三栏框架（对应 DSH AppFrame：sidebar / main / details）
export function AppFrame() {
  const [view, setView] = useState<View>('members')
  const [selected, setSelected] = useState<Member | null>(null)
  const [members, setMembers] = useState<Member[]>([])
  const [sharing, setSharing] = useState(true)
  const [error, setError] = useState('')

  // 轮询刷新成员/用量（对应 DSH 会话列表刷新语义）
  const refresh = useCallback(async () => {
    try {
      const resp = await fetch('/api/members')
      if (resp.ok) {
        const data = await resp.json()
        setMembers(data.members || [])
        setError('')
      }
    } catch (e) {
      setError('无法连接组长端服务')
    }
  }, [])

  useEffect(() => {
    refresh()
    const t = setInterval(refresh, 5000)
    return () => clearInterval(t)
  }, [refresh])

  const selectMember = (m: Member) => {
    setSelected(m)
    setView('details')
  }

  return (
    <div className={styles.frame}>
      <Sidebar view={view} setView={setView} sharing={sharing} setSharing={setSharing} />
      <main className={styles.main}>
        {error && <div className={styles.error}>{error}</div>}
        {view === 'members' && <MemberList members={members} onSelect={selectMember} />}
        {view === 'usage' && <UsageTable members={members} />}
        {view === 'controls' && <ControlsPanel sharing={sharing} setSharing={setSharing} />}
        {view === 'details' && selected && <DetailsPanel member={selected} onBack={() => setView('members')} />}
      </main>
      <aside className={styles.details}>
        {selected ? <DetailsPanel member={selected} onBack={() => setView('members')} /> : <div className={styles.detailsEmpty}>选中成员查看详情</div>}
      </aside>
    </div>
  )
}