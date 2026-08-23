import styles from './MemberList.module.css'
import type { Member } from './types'
import { L } from './types'

interface Props { members: Member[]; onSelect: (m: Member) => void }

// 主区：成员列表（对应 DSH 会话列表）
export function MemberList({ members, onSelect }: Props) {
  const t = L.zh
  return (
    <div>
      <h2 className={styles.title}>{t.memberList} ({members.length})</h2>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>{t.displayName}</th>
            <th>{t.machineName}</th>
            <th>{t.ip}</th>
            <th>{t.status}</th>
            <th>{t.usage}</th>
            <th>{t.actions}</th>
          </tr>
        </thead>
        <tbody>
          {members.map(m => (
            <tr key={m.memberId} onClick={() => onSelect(m)}>
              <td className={styles.name}>{m.displayName}</td>
              <td>{m.machineName}</td>
              <td>{m.ip || '-'}</td>
              <td><span className={m.online ? styles.online : styles.offline}>{m.online ? t.online : t.offline}</span></td>
              <td>{m.usage?.totalTokens ?? 0}</td>
              <td><button className={styles.kickBtn} onClick={(e) => { e.stopPropagation(); kick(m) }}>{t.kick}</button></td>
            </tr>
          ))}
          {members.length === 0 && <tr><td colSpan={6} className={styles.empty}>暂无成员</td></tr>}
        </tbody>
      </table>
    </div>
  )
}

async function kick(m: Member) {
  await fetch('/api/control', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'revoke', memberId: m.memberId, ip: m.ip }),
  })
}