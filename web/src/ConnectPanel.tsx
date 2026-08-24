import { useEffect, useState } from 'react'
import styles from './ConnectPanel.module.css'
import { useT } from './types'

interface Info {
  port: number
  lanIp: string
  baseUrl: string
  anthropicBaseUrl: string
  consoleUrl: string
  localOnly: boolean
  models: string[]
}

// 接入信息页：网关对外地址 / 令牌 / 模型清单 / cc-switch 配置指引
export function ConnectPanel() {
  const t = useT()
  const [info, setInfo] = useState<Info | null>(null)
  const [copied, setCopied] = useState('')

  useEffect(() => {
    fetch('/api/info')
      .then(r => (r.ok ? r.json() : null))
      .then(d => { if (d) setInfo(d) })
      .catch(() => {})
  }, [])

  if (!info) {
    return (
      <div className={styles.wrap}>
        <h2 className={styles.title}>{t.connectTitle}</h2>
        <div className={styles.card}>
          <div className={styles.muted}>…</div>
        </div>
      </div>
    )
  }

  const copy = async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(key)
      setTimeout(() => setCopied(''), 1500)
    } catch { /* clipboard unavailable */ }
  }

  const tokenCmd = 'curl.exe -X POST ' + info.consoleUrl + '/auth/token' +
    ' -H "Content-Type: application/json" -d "' + JSON.stringify({ machineName: 'my-pc' }) + '"'
  const anthroCmd = [ 'set ANTHROPIC_BASE_URL=' + info.anthropicBaseUrl,
    'set ANTHROPIC_AUTH_TOKEN=<member-token>' ].join('\n')

  return (
    <div className={styles.wrap}>
      <h2 className={styles.title}>{t.connectTitle}</h2>
      <div className={styles.subtitle}>{t.connectSubtitle}</div>
      {info.localOnly && <div className={styles.localOnly}>{t.connLocalOnly}</div>}

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connConsole}</div>
        <div className={styles.row}>
          <code className={styles.mono}>{info.consoleUrl}</code>
          <button className={styles.copyBtn} onClick={() => copy('console', info.consoleUrl)}>
            {copied === 'console' ? t.connCopied : t.connCopy}
          </button>
        </div>
        <div className={styles.meta}>
          {t.connLanIp}: <code>{info.lanIp}</code> · {t.connPort}: <code>{info.port}</code>
        </div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connEndpoint}</div>
        <div className={styles.row}>
          <code className={styles.mono}>{info.baseUrl}</code>
          <button className={styles.copyBtn} onClick={() => copy('url', info.baseUrl)}>
            {copied === 'url' ? t.connCopied : t.connCopy}
          </button>
        </div>
        <div className={styles.hint}>{t.connOpenaiNote}</div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connAnthroTitle}</div>
        <div className={styles.meta} style={{ marginTop: 0, marginBottom: 8 }}>{t.connAnthroEndpoint}</div>
        <div className={styles.row}>
          <code className={styles.mono}>{info.anthropicBaseUrl}</code>
          <button className={styles.copyBtn} onClick={() => copy('anthro', info.anthropicBaseUrl)}>
            {copied === 'anthro' ? t.connCopied : t.connCopy}
          </button>
        </div>
        <div className={styles.hint}>{t.connAnthroEnv}</div>
        <div className={styles.row}>
          <code className={styles.monoBlock}>{anthroCmd}</code>
          <button className={styles.copyBtn} onClick={() => copy('anthroCmd', anthroCmd)}>
            {copied === 'anthroCmd' ? t.connCopied : t.connCopy}
          </button>
        </div>
        <div className={styles.hint}>{t.connAnthroLinux} · {t.connAnthroTokenHint}</div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connTokenTitle}</div>
        <div className={styles.row}>
          <code className={styles.monoBlock}>{tokenCmd}</code>
          <button className={styles.copyBtn} onClick={() => copy('token', tokenCmd)}>
            {copied === 'token' ? t.connCopied : t.connCopy}
          </button>
        </div>
        <div className={styles.hint}>{t.connTokenHint}</div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connModelsTitle}</div>
        {info.models.length === 0 ? (
          <div className={styles.muted}>{t.connNoModels}</div>
        ) : (
          <div className={styles.chips}>
            {info.models.map(m => <span className={styles.modelChip} key={m}>{m}</span>)}
          </div>
        )}
      </div>

      <div className={styles.card}>
        <div className={styles.cardTitle}>{t.connCcTitle}</div>
        <table className={styles.ccTable}>
          <tbody>
            <tr>
              <td className={styles.ccKey}>{t.connCcName}</td>
              <td><code className={styles.mono}>AIPowerLink</code></td>
            </tr>
            <tr>
              <td className={styles.ccKey}>{t.connCcApi}</td>
              <td><code className={styles.mono}>{info.baseUrl}</code></td>
            </tr>
            <tr>
              <td className={styles.ccKey}>{t.connCcKey}</td>
              <td><code className={styles.mono}>（{t.connTokenTitle}）</code></td>
            </tr>
          </tbody>
        </table>
        <div className={styles.hint}>{t.connMemberNote}</div>
      </div>

    </div>
  )
}
