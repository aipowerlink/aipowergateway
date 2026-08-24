import { useState, useContext } from 'react'
import styles from './Sidebar.module.css'
import type { Lang, View } from './types'
import { LangContext } from './types'
import { L } from './types'

interface Props {
  view: View
  setView: (v: View) => void
  sharing: boolean
  setSharing: (s: boolean) => void
}

// 左侧导航栏（对应 DSH ui-sidebar：brand + nav + settings 入口）
export function Sidebar({ view, setView, sharing, setSharing }: Props) {
  const { lang, setLang } = useContext(LangContext)
  const t = L[lang]

  const toggleSharing = async () => {
    const action = sharing ? 'pause' : 'resume'
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action }),
    })
    if (resp.ok) setSharing(!sharing)
  }

  return (
    <nav className={styles.sidebar}>
      <div className={styles.brand}>
        <span className={styles.brandMark}>⚡</span>
        <span className={styles.brandName}>{t.appName}</span>
      </div>
      <div className={styles.nav}>
        <button className={view === 'members' ? styles.navActive : styles.navItem} onClick={() => setView('members')}>
          {t.navMembers}
        </button>
        <button className={view === 'usage' ? styles.navActive : styles.navItem} onClick={() => setView('usage')}>
          {t.navUsage}
        </button>
        <button className={view === 'controls' ? styles.navActive : styles.navItem} onClick={() => setView('controls')}>
          {t.navControls}
        </button>
        <button className={view === 'models' ? styles.navActive : styles.navItem} onClick={() => setView('models')}>
          {t.navModels}
        </button>
      </div>
      <div className={styles.footer}>
        <div className={styles.sharingStatus}>
          <span className={sharing ? styles.dotOn : styles.dotOff} />
          {sharing ? t.sharingOn : t.sharingOff}
        </div>
        <button className={styles.sharingBtn} onClick={toggleSharing}>
          {sharing ? t.pauseSharing : t.startSharing}
        </button>
        <select className={styles.langSelect} value={lang} onChange={(e) => setLang(e.target.value as Lang)}>
          <option value="zh">中文</option>
          <option value="en">English</option>
        </select>
      </div>
    </nav>
  )
}