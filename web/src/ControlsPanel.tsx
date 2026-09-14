import { useState, useEffect, useCallback } from 'react'
import styles from './ControlsPanel.module.css'
import { useT } from './types'

interface Props { sharing: boolean; setSharing: (s: boolean) => void }

interface RuleSetSummary { name: string; version: number; ruleCount: number }

// 管理操作面板（共享开关 + 开机启动 + 版本信息 + 规则执行引擎）
export function ControlsPanel({ sharing, setSharing }: Props) {
  const t = useT()
  const [msg, setMsg] = useState('')
  const [autostart, setAutostart] = useState(false)
  const [info, setInfo] = useState<{ version?: string; github?: string } | null>(null)
  // 链路加密组长端策略（M3）：off | aes-gcm | enforce
  const [linkEncrypt, setLinkEncrypt] = useState<string>('aes-gcm')
  // 负载红线拦截（挖矿/深伪）
  const [loadPolicy, setLoadPolicy] = useState(true)
  const [loadPolicyHits, setLoadPolicyHits] = useState<{ mining: number; deepfake: number }>({ mining: 0, deepfake: 0 })
  // 规则执行引擎（智能路由）
  const [ruleSets, setRuleSets] = useState<RuleSetSummary[]>([])
  const [ruleEditor, setRuleEditor] = useState('')
  const [ruleNames, setRuleNames] = useState<string[]>([])

  // 读取版本/GitHub/开机启动/链路加密/负载红线状态（/api/info）
  useEffect(() => {
    fetch('/api/info')
      .then(r => r.json())
      .then(d => {
        setInfo({ version: d.version, github: d.github })
        if (typeof d.autostart === 'boolean') setAutostart(d.autostart)
        if (typeof d.linkEncrypt === 'string') setLinkEncrypt(d.linkEncrypt)
        if (typeof d.loadPolicy === 'boolean') setLoadPolicy(d.loadPolicy)
        if (d.loadPolicyHits) setLoadPolicyHits({ mining: d.loadPolicyHits.mining || 0, deepfake: d.loadPolicyHits.deepfake || 0 })
      })
      .catch(() => {})
    refreshRules()
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

  const saveLinkEncrypt = useCallback(async (mode: string) => {
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'link-encrypt', mode }),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok && typeof data.linkEncrypt === 'string') {
      setLinkEncrypt(data.linkEncrypt)
      setMsg(t.encSaved + '（' + data.linkEncrypt + '）')
    } else {
      setMsg(t.encSavingFail + (data.error?.message || resp.status))
    }
  }, [t])

  const toggleLoadPolicy = useCallback(async (enabled: boolean) => {
    const resp = await fetch('/api/control', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'load-policy', enabled }),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok && typeof data.loadPolicy === 'boolean') {
      setLoadPolicy(data.loadPolicy)
      setLoadPolicyHits({ mining: data.miningHits || 0, deepfake: data.deepfakeHits || 0 })
      setMsg(t.loadPolicySaved + '（' + (data.loadPolicy ? t.loadPolicyOn : t.loadPolicyOff) + '）')
    } else {
      setMsg(t.loadPolicySavingFail + (data.error?.message || resp.status))
    }
  }, [t])

  // 刷新规则引擎状态（/api/rules：已加载规则集 + 可用规则名）
  const refreshRules = useCallback(async () => {
    try {
      const resp = await fetch('/api/rules')
      const data = await resp.json().catch(() => ({}))
      const sets: RuleSetSummary[] = (data.ruleSets || []).map((rs: Record<string, unknown>) => ({
        name: String(rs.name || '?'),
        version: Number(rs.version || 0),
        ruleCount: Array.isArray(rs.rules) ? rs.rules.length : 0,
      }))
      setRuleSets(sets)
      setRuleNames(data.ruleNames || [])
      if (sets.length > 0) setRuleEditor(JSON.stringify((data.ruleSets || []), null, 2))
    } catch {
      /* 服务未就绪时静默 */
    }
  }, [])

  // 保存规则集（POST /api/rules → model-rule-set.json 落盘 + 热加载）
  const saveRules = useCallback(async () => {
    let parsed: unknown
    try {
      parsed = JSON.parse(ruleEditor)
    } catch (e) {
      setMsg(t.rulesSaveFail + String(e))
      return
    }
    const body = Array.isArray(parsed) ? { ruleSets: parsed } : parsed
    const resp = await fetch('/api/rules', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    const data = await resp.json().catch(() => ({}))
    if (resp.ok) {
      setMsg(t.rulesSaved + '（' + (data.ruleNames || []).join('、') + '）')
      refreshRules()
    } else {
      setMsg(t.rulesSaveFail + (data.error?.message || resp.status))
    }
  }, [ruleEditor, refreshRules, t])

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
        <h3>{t.encTitle}</h3>
        <p className={styles.desc}>{t.encHint}</p>
        <div className={styles.row}>
          {[
            ['off', t.encOff],
            ['aes-gcm', t.encAesGcm],
            ['enforce', t.encEnforce],
          ].map(([mode, label]) => (
            <button
              key={mode}
              className={styles.btn}
              disabled={linkEncrypt === mode}
              onClick={() => saveLinkEncrypt(mode)}
              title={label}
            >
              {label}
            </button>
          ))}
        </div>
        <p className={styles.desc}>当前策略：{linkEncrypt}</p>
      </div>

      <div className={styles.card}>
        <h3>{t.loadPolicyTitle}</h3>
        <p className={styles.desc}>{t.loadPolicyHint}</p>
        <label className={styles.switchRow}>
          <input
            type="checkbox"
            className={styles.switch}
            checked={loadPolicy}
            onChange={e => toggleLoadPolicy(e.target.checked)}
          />
          <span className={styles.switchLabel}>{loadPolicy ? t.loadPolicyOn : t.loadPolicyOff}</span>
        </label>
        <p className={styles.desc}>
          {t.loadPolicyMining}: {loadPolicyHits.mining} ｜ {t.loadPolicyDeepfake}: {loadPolicyHits.deepfake}
        </p>
      </div>

      <div className={styles.card}>
        <h3>{t.rulesTitle}</h3>
        <p className={styles.desc}>{t.rulesHint}</p>
        <p className={styles.desc}>
          {t.rulesRuleNames}:{' '}
          <span className={styles.code}>{ruleNames.length > 0 ? ruleNames.join('、') : '–'}</span>
        </p>
        <p className={styles.desc}>
          {t.rulesLoaded}:{' '}
          {ruleSets.length > 0
            ? ruleSets.map(rs => `${rs.name} v${rs.version}（${rs.ruleCount} ${t.rulesCount}）`).join('；')
            : '–'}
        </p>
        <p className={styles.desc}>{t.rulesEditorHint}</p>
        <textarea
          className={styles.jsonArea}
          rows={8}
          value={ruleEditor}
          onChange={e => setRuleEditor(e.target.value)}
          placeholder='{"ruleSets":[]}'
        />
        <div className={styles.row}>
          <button className={styles.btn} onClick={saveRules}>
            {t.rulesSave}
          </button>
        </div>
        <p className={styles.desc}>{t.rulesUsageTip}</p>
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
