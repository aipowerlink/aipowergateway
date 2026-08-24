import { useState } from 'react'
import styles from './UsageTable.module.css'
import type { Member } from './types'
import { useT } from './types'

interface Props {
  members: Member[]
  quotas: Record<string, number>
  onSetQuota: (memberId: string, quota: number) => void
}

// 行内配额编辑器：输入数字失焦保存，清空/0 = 解除限制
function QuotaEditor({ quota, onSave }: { quota?: number; onSave: (q: number) => void }) {
  const [draft, setDraft] = useState('')
  const save = () => {
    const v = parseInt(draft, 10)
    onSave(Number.isFinite(v) && v > 0 ? v : 0)
    setDraft('')
  }
  return (
    <input
      className={styles.quotaInput}
      type="number"
      min={0}
      step={1000}
      placeholder={quota ? String(quota) : '∞'}
      value={draft}
      onChange={e => setDraft(e.target.value)}
      onBlur={save}
      onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur() }}
    />
  )
}

// 用量统计（主区视图）：总量 + 配额状态 + CSV 导出
export function UsageTable({ members, quotas, onSetQuota }: Props) {
  const t = useT()
  const sorted = [...members].sort((a, b) => (b.usage?.totalTokens ?? 0) - (a.usage?.totalTokens ?? 0))
  const quotaCell = (m: Member) => {
    const q = quotas[m.memberId]
    const used = m.usage?.totalTokens ?? 0
    if (!q) return <span className={styles.unlimited}>{t.unlimited}</span>
    const over = used >= q
    return <span className={over ? styles.over : styles.ok}>{used.toLocaleString()} / {q.toLocaleString()}</span>
  }
  return (
    <div>
      <div className={styles.toolbar}>
        <h2 className={styles.title}>{t.usageTable}</h2>
        <a className={styles.exportBtn} href="/api/usage/export" download="usage.csv">{t.exportCsv}</a>
      </div>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>{t.displayName}</th>
            <th>{t.machineName}</th>
            <th>{t.totalTokens}</th>
            <th>Prompt</th>
            <th>Completion</th>
            <th>{t.calls}</th>
            <th>{t.quota}</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map(m => (
            <tr key={m.memberId}>
              <td className={styles.name}>{m.displayName}</td>
              <td>{m.machineName}</td>
              <td className={styles.tokens}>{m.usage?.totalTokens ?? 0}</td>
              <td>{m.usage?.promptTokens ?? 0}</td>
              <td>{m.usage?.completionTokens ?? 0}</td>
              <td>{m.usage?.calls ?? 0}</td>
              <td>
                <span className={styles.quotaRow}>
                  <QuotaEditor quota={quotas[m.memberId]} onSave={q => onSetQuota(m.memberId, q)} />
                  <span>{quotaCell(m)}</span>
                </span>
              </td>
            </tr>
          ))}
          {members.length === 0 && <tr><td colSpan={7} className={styles.empty}>暂无数据</td></tr>}
        </tbody>
      </table>
    </div>
  )
}
