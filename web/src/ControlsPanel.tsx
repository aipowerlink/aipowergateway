import { useState, useEffect, useCallback } from 'react'
import styles from './ControlsPanel.module.css'
import { useT } from './types'

interface Props { sharing: boolean; setSharing: (s: boolean) => void }

// 管理操作面板（共享开关 + 开机启动 + 版本信息）
export function ControlsPanel({ sharing, setSharing }: Props) {
  const t = useT()
  const [msg, setMsg] = useState('')
  const [autostart, setAutostart] = useState(false)
  const [info, setInfo] = useState<{ version?: string; github?: string } | null>(null)

  // 读取版本/GitHub/开机启动状态（/api/info）
  useEffect(() => {
    fetch('/api/info')
      .then(r => r.json())
      .then(d => {
        setInfo({ version: d.version, github: d.github })
        if (typeof d.autostart === 'boolean') setAutostart(d.autostart)
      })
      .catch(() => {})
  }, [])

  const doControl = async (action: string, extra: Record<string, string> = {}) => {
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action, ...extra }),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok) {
      setMsg(`${t.controls} OK${data.sharing !== undefined ? '（' + t.controls + '=' + (data.sharing ? t.sharingOn : t.sharingOff) + '）' : ''}`)
      if (data.sharing !== undefined) setSharing(data.sharing)
    } else {
      setMsg(t.autostartFailed + ': ' + (data.error?.message || resp.status))
    }
  }

  const toggleAutostart = useCallback(async (enabled: boolean) => {
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'autostart', enabled }),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok && typeof data.autostart === 'boolean') {
      setAutostart(data.autostart)
      setMsg(t.autostartTitle + ': ' + (data.autostart ? t.autostartOn : t.autostartOff))
    } else {
      setMsg(t.autostartFailed + ': ' + (data.error?.message || resp.status))
    }
  }, [t])

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
        <h3>{t.autostartTitle}</h3>
        <p className={styles.desc}>{t.autostartHint}</p>
        <label className={styles.switchRow}>
          <input
            type="checkbox"
            className={styles.switch}
            checked={autostart}
            onChange={e => toggleAutostart(e.target.checked)}
          />
          <span className={styles.switchLabel}>{autostart ? t.autostartOn : t.autostartOff}</span>
        </label>
      </div>

      <div className={styles.card}>
        <h3>{t.aboutTitle}</h3>
        <p className={styles.desc}>
          {t.versionLabel}: <span className={styles.code}>{info?.version || '–'}</span>
        </p>
        <p className={styles.desc}>
          {t.githubLabel}:{' '}
          <a className={styles.link} href={t.githubHref} target="_blank" rel="noopener noreferrer">
            {t.githubHref}
          </a>
        </p>
      </div>

      {msg && <div className={styles.msg}>{msg}</div>}
    </div>
  )
}
