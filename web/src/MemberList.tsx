import styles from './MemberList.module.css'
import type { Member } from './types'
import { useT } from './types'

interface Props { members: Member[]; onSelect: (m: Member) => void }

// 主区：成员列表（对应 DSH 会话列表）
export function MemberList({ members, onSelect }: Props) {
  const t = useT()
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
              <td>{m.machineName}{m.isLocal && <span className={styles.localBadge}>{t.memberLocal}</span>}</td>
              <td>{m.ip || '-'}</td>
              <td>
                {m.banned
                  ? <span className={styles.banned}>{t.banned}</span>
                  : <span className={m.online ? styles.online : styles.offline}>{m.online ? t.online : t.offline}</span>}
              </td>
              <td>{m.usage?.totalTokens ?? 0}</td>
              <td><button className={m.banned ? styles.unbanBtn : styles.kickBtn} onClick={(e) => { e.stopPropagation(); m.banned ? unban(m) : kick(m) }}>{m.banned ? t.unban : t.kick}</button></td>
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

async function unban(m: Member) {
  await fetch('/api/control', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'unban', memberId: m.memberId, ip: m.ip }),
  })
}