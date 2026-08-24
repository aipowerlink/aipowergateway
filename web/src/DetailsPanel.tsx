import styles from './DetailsPanel.module.css'
import type { Member } from './types'
import { useT } from './types'

interface Props { member: Member; onBack?: () => void }

// 右栏：成员详情（对应 DSH DetailsPanel）
export function DetailsPanel({ member, onBack }: Props) {
  const t = useT()
  const fmt = (ts: number) => ts ? new Date(ts * 1000).toLocaleString() : '-'
  return (
    <div className={styles.panel}>
      {onBack && <button className={styles.back} onClick={onBack}>← {t.back}</button>}
      <h3 className={styles.title}>{t.details}</h3>
      <div className={styles.row}><span className={styles.label}>{t.displayName}</span><span>{member.displayName}</span></div>
      <div className={styles.row}><span className={styles.label}>{t.machineName}</span><span>{member.machineName}</span></div>
      <div className={styles.row}><span className={styles.label}>{t.ip}</span><span>{member.ip || '-'}</span></div>
      {member.gatewayId && <div className={styles.row}><span className={styles.label}>{t.gateway}</span><span>{member.gatewayId}</span></div>}
      <div className={styles.row}><span className={styles.label}>{t.status}</span>
        {member.banned
          ? <span className={styles.banned}>{t.banned}</span>
          : <span className={member.online ? styles.online : styles.offline}>{member.online ? t.online : t.offline}</span>}
      </div>
      <div className={styles.row}><span className={styles.label}>{t.usage}</span><span>{member.usage?.totalTokens ?? 0} tokens</span></div>
      <div className={styles.row}><span className={styles.label}>{t.calls}</span><span>{member.usage?.calls ?? 0}</span></div>
      <div className={styles.row}><span className={styles.label}>{t.joinedAt}</span><span>{fmt(member.joinedAt)}</span></div>
      <div className={styles.row}><span className={styles.label}>{t.lastSeen}</span><span>{fmt(member.lastSeen)}</span></div>
      {member.usage?.modelTokens && Object.keys(member.usage.modelTokens).length > 0 && (
        <div className={styles.models}>
          <div className={styles.modelsTitle}>{t.models}</div>
          {Object.entries(member.usage.modelTokens)
            .sort((a, b) => b[1] - a[1])
            .map(([model, tokens]) => (
              <div className={styles.row} key={model}><span className={styles.label}>{model}</span><span>{tokens.toLocaleString()} tokens</span></div>
            ))}
        </div>
      )}
    </div>
  )
}