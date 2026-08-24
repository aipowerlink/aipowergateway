import { useState } from 'react'
import styles from './DetailsPanel.module.css'
import type { Member } from './types'
import { useT } from './types'

interface Props { member: Member; onBack?: () => void; onRename?: (memberId: string, displayName: string) => Promise<void> }

// 右栏：成员详情（对应 DSH DetailsPanel）
export function DetailsPanel({ member, onBack, onRename }: Props) {
  const t = useT()
  const fmt = (ts: number) => ts ? new Date(ts * 1000).toLocaleString() : '-'
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(member.displayName)
  const [saving, setSaving] = useState(false)
  const saveRename = async () => {
    const name = draft.trim()
    if (!name || !onRename) return
    setSaving(true)
    try {
      await onRename(member.memberId, name)
      setEditing(false)
    } finally {
      setSaving(false)
    }
  }
  return (
    <div className={styles.panel}>
      {onBack && <button className={styles.back} onClick={onBack}>← {t.back}</button>}
      <h3 className={styles.title}>{t.details}</h3>
      <div className={styles.row}>
        <span className={styles.label}>{t.displayName}</span>
        {editing ? (
          <span className={styles.renameWrap}>
            <input className={styles.renameInput} value={draft} onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') saveRename(); if (e.key === 'Escape') setEditing(false) }} autoFocus />
            <button className={styles.renameBtn} onClick={saveRename} disabled={saving}>{t.save}</button>
            <button className={styles.renameCancel} onClick={() => setEditing(false)}>{t.cancel}</button>
          </span>
        ) : (
          <span className={styles.renameRow}>
            <span>{member.displayName}</span>
            <button className={styles.editBtn} onClick={() => { setDraft(member.displayName); setEditing(true) }}>{t.edit}</button>
          </span>
        )}
      </div>
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