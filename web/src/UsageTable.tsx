import styles from './UsageTable.module.css'
import type { Member } from './types'
import { useT } from './types'

interface Props { members: Member[] }

// 用量统计（主区视图）
export function UsageTable({ members }: Props) {
  const t = useT()
  const sorted = [...members].sort((a, b) => (b.usage?.totalTokens ?? 0) - (a.usage?.totalTokens ?? 0))
  return (
    <div>
      <h2 className={styles.title}>{t.usageTable}</h2>
      <table className={styles.table}>
        <thead>
          <tr>
            <th>{t.displayName}</th>
            <th>{t.machineName}</th>
            <th>{t.totalTokens}</th>
            <th>Prompt</th>
            <th>Completion</th>
            <th>{t.calls}</th>
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
            </tr>
          ))}
          {members.length === 0 && <tr><td colSpan={6} className={styles.empty}>暂无数据</td></tr>}
        </tbody>
      </table>
    </div>
  )
}