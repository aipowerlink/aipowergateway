import { useState, useEffect, useCallback } from 'react'
import styles from './AppFrame.module.css'
import { Sidebar } from './Sidebar'
import { MemberList } from './MemberList'
import { UsageTable } from './UsageTable'
import { ControlsPanel } from './ControlsPanel'
import { BackendsPanel } from './BackendsPanel'
import { ConnectPanel } from './ConnectPanel'
import { DetailsPanel } from './DetailsPanel'
import { LangContext, type Member, View } from './types'

// 三栏框架（对应 DSH AppFrame：sidebar / main / details）
export function AppFrame() {
  const [view, setView] = useState<View>('members')
  const [selected, setSelected] = useState<Member | null>(null)
  const [members, setMembers] = useState<Member[]>([])
  const [quotas, setQuotas] = useState<Record<string, number>>({})
  const [lang, setLang] = useState<'zh' | 'en'>('en')
  const [sharing, setSharing] = useState(true)
  const [error, setError] = useState('')

  // 轮询刷新成员/用量（对应 DSH 会话列表刷新语义）
  const refresh = useCallback(async () => {
    try {
      const [m, q] = await Promise.all([
        fetch('/api/members').then(r => (r.ok ? r.json() : null)),
        fetch('/api/quota').then(r => (r.ok ? r.json() : null)),
      ])
      if (m) {
        setMembers(m.members || [])
        setError('')
      }
      if (q?.quotas) {
        const map: Record<string, number> = {}
        for (const row of q.quotas) map[row.memberId] = row.quota
        setQuotas(map)
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

  const setQuota = useCallback(async (memberId: string, quota: number) => {
    try {
      await fetch('/api/quota', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ memberId, quota }),
      })
    } catch (e) {
      setError('配额保存失败')
    }
    await refresh()
  }, [refresh])

  const selectMember = (m: Member) => {
    setSelected(m)
    setView('details')
  }

  const renameMember = useCallback(async (memberId: string, displayName: string) => {
    try {
      await fetch('/api/control', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'rename', memberId, displayName }),
      })
    } catch (e) {
      setError('改名失败')
    }
    setSelected((m) => (m && m.memberId === memberId ? { ...m, displayName } : m))
    await refresh()
  }, [refresh])

  return (
    <LangContext.Provider value={{ lang, setLang }}>
    <div className={styles.frame}>
      <Sidebar view={view} setView={setView} sharing={sharing} setSharing={setSharing} />
      <main className={styles.main}>
        {error && <div className={styles.error}>{error}</div>}
        {view === 'members' && <MemberList members={members} onSelect={selectMember} />}
        {view === 'usage' && <UsageTable members={members} quotas={quotas} onSetQuota={setQuota} />}
        {view === 'controls' && <ControlsPanel sharing={sharing} setSharing={setSharing} />}
        {view === 'models' && <BackendsPanel />}
        {view === 'connect' && <ConnectPanel />}
        {view === 'details' && selected && <DetailsPanel member={selected} onBack={() => setView('members')} onRename={renameMember} />}
      </main>
      <aside className={styles.details}>
        {selected ? <DetailsPanel member={selected} onBack={() => setView('members')} onRename={renameMember} /> : <div className={styles.detailsEmpty}>选中成员查看详情</div>}
      </aside>
    </div>
    </LangContext.Provider>
  )
}