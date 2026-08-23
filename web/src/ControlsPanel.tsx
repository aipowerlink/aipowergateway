import { useState } from 'react'
import styles from './ControlsPanel.module.css'
import { useT } from './types'

interface Props { sharing: boolean; setSharing: (s: boolean) => void }

// 管理操作面板
export function ControlsPanel({ sharing, setSharing }: Props) {
  const t = useT()
  const [pw, setPw] = useState('')
  const [msg, setMsg] = useState('')

  const doControl = async (action: string, extra: Record<string, string> = {}) => {
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action, ...extra }),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok) {
      setMsg(`操作成功${data.sharing !== undefined ? '（共享=' + (data.sharing ? '开' : '关') + '）' : ''}`)
      if (data.sharing !== undefined) setSharing(data.sharing)
    } else {
      setMsg('操作失败: ' + (data.error?.message || resp.status))
    }
  }

  return (
    <div>
      <h2 className={styles.title}>{t.controls}</h2>
      <div className={styles.card}>
        <h3>{t.navControls}</h3>
        <p className={styles.desc}>当前共享状态：{sharing ? t.sharingOn : t.sharingOff}</p>
        <button className={styles.btn} onClick={() => doControl(sharing ? 'pause' : 'resume')}>
          {sharing ? t.pauseSharing : t.startSharing}
        </button>
      </div>
      <div className={styles.card}>
        <h3>{t.changePassword}</h3>
        <div className={styles.row}>
          <input type="password" placeholder={t.newPassword} value={pw} onChange={(e) => setPw(e.target.value)} className={styles.input} />
          <button className={styles.btn} onClick={() => doControl('changePassword', { password: pw })} disabled={!pw}>
            {t.changePassword}
          </button>
        </div>
      </div>
      {msg && <div className={styles.msg}>{msg}</div>}
    </div>
  )
}