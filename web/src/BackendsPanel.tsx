import { useCallback, useEffect, useState } from 'react'
import styles from './BackendsPanel.module.css'
import { useT, type BackendRow } from './types'

interface FormState {
  editingId: string | null
  provider: string
  apiKey: string
  apiKeyEnv: string
  model: string
  baseUrl: string
}

const emptyForm: FormState = { editingId: null, provider: 'deepseek', apiKey: '', apiKeyEnv: '', model: '', baseUrl: '' }

// 模型设置（对齐 DeepSeek Harness 的 settings → models 面板）
export function BackendsPanel() {
  const t = useT()
  const [rows, setRows] = useState<BackendRow[]>([])
  const [form, setForm] = useState<FormState>(emptyForm)
  const [formOpen, setFormOpen] = useState(false)
  const [msg, setMsg] = useState('')
  const [err, setErr] = useState('')
  const [loading, setLoading] = useState(true)

  const load = useCallback(async () => {
    try {
      const resp = await fetch('/api/backends')
      const data = await resp.json().catch(() => ({}))
      setRows(data.backends || [])
      setLoading(false)
    } catch {
      setLoading(false)
    }
  }, [])

  useEffect(() => { load() }, [load])

  const startAdd = (custom: boolean) => {
    setForm({ ...emptyForm, provider: custom ? 'custom' : 'deepseek' })
    setFormOpen(true)
    setMsg('')
    setErr('')
  }

  const startEdit = (row: BackendRow) => {
    setForm({
      editingId: row.id,
      provider: row.provider,
      apiKey: '',
      apiKeyEnv: '',
      model: row.model,
      baseUrl: row.baseUrl,
    })
    setFormOpen(true)
    setMsg('')
    setErr('')
  }

  const submit = async () => {
    if (form.provider === 'custom' && (!form.baseUrl.trim() || !form.model.trim())) {
      setErr(t.invalidCustom)
      return
    }
    const body: Record<string, string> = { provider: form.provider }
    if (form.editingId) body.id = form.editingId
    if (form.apiKey.trim()) body.apiKey = form.apiKey.trim()
    if (form.apiKeyEnv.trim()) body.apiKeyEnv = form.apiKeyEnv.trim()
    if (form.model.trim()) body.model = form.model.trim()
    if (form.baseUrl.trim()) body.baseUrl = form.baseUrl.trim()
    try {
      const resp = await fetch('/api/backends', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
      const data = await resp.json().catch(() => ({}))
      if (resp.ok) {
        setMsg(t.saved + ' ' + (data.backend?.provider || form.provider))
        setFormOpen(false)
        setForm(emptyForm)
        await load()
      } else {
        setErr(data.error?.message || String(resp.status))
      }
    } catch (e) {
      setErr(String(e))
    }
  }

  const del = async (row: BackendRow) => {
    try {
      await fetch('/api/backends/' + encodeURIComponent(row.id), { method: 'DELETE' })
      await load()
    } catch (e) {
      setErr(String(e))
    }
  }

  const keyLabel = (r: BackendRow) => {
    if (r.keySource === 'env') return t.apiKeyConfigured + ' · ' + t.keySourceEnv + ' ' + r.maskedKey
    if (r.keySource === 'file') return t.apiKeyConfigured
    return r.maskedKey ? t.apiKeyMissing + ' · ' + r.maskedKey : t.apiKeyMissing
  }

  return (
    <div>
      <h2 className={styles.title}>{t.modelsTitle}</h2>
      <p className={styles.hint}>{t.modelsHint}</p>
      <div className={styles.toolbar}>
        <button className={styles.btn} onClick={() => startAdd(false)}>{t.addProvider}</button>
        <button className={styles.btn} onClick={() => startAdd(true)}>{t.addCustomProvider}</button>
      </div>

      {loading && <div className={styles.empty}>{t.loading}</div>}
      {!loading && rows.length === 0 && <div className={styles.empty}>{t.emptyBackends}</div>}

      {rows.map((row) => (
        <div className={styles.card} key={row.id}>
          <div className={styles.cardHead}>
            <span className={styles.providerName}>{row.provider}</span>
            <span className={row.keySource === 'none' ? styles.badgeMissing : styles.badge}>{keyLabel(row)}</span>
            {row.model && <span className={styles.modelChip}>{row.model}</span>}
            <div className={styles.cardActions}>
              <button className={styles.btn} onClick={() => startEdit(row)}>{t.edit}</button>
              <button className={styles.btnDanger} onClick={() => del(row)}>{t.delete}</button>
            </div>
          </div>
          {row.baseUrl && <div className={styles.url}>{row.baseUrl}</div>}
        </div>
      ))}

      {formOpen && (
        <div className={styles.card}>
          <h3 className={styles.formTitle}>{form.editingId ? t.edit + ' — ' + form.provider : t.addProvider}</h3>
          {!form.editingId && (
            <label className={styles.field}>
              <span>{t.provider}</span>
              <select className={styles.input} value={form.provider} onChange={(e) => setForm({ ...form, provider: e.target.value })}>
                <option value="deepseek">DeepSeek</option>
                <option value="kimi">Kimi</option>
                <option value="zhipu">Zhipu</option>
                <option value="mock">Mock（本地验证）</option>
                <option value="custom">{t.providerCustom}</option>
              </select>
            </label>
          )}
          <label className={styles.field}>
            <span>{t.apiKeyLabel}</span>
            <input className={styles.input} type="password" value={form.apiKey} placeholder="sk-..."
              onChange={(e) => setForm({ ...form, apiKey: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>{t.apiKeyEnvLabel}</span>
            <input className={styles.input} value={form.apiKeyEnv} placeholder="AIPOWERLINK_DEEPSEEK_API_KEY"
              onChange={(e) => setForm({ ...form, apiKeyEnv: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>{t.modelLabel}</span>
            <input className={styles.input} value={form.model} placeholder={form.provider === 'deepseek' ? 'deepseek-chat' : ''}
              onChange={(e) => setForm({ ...form, model: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>{t.baseUrlLabel}</span>
            <input className={styles.input} value={form.baseUrl} placeholder="https://api.deepseek.com"
              onChange={(e) => setForm({ ...form, baseUrl: e.target.value })} />
            {form.provider !== 'custom' && <em className={styles.hintSmall}>{t.customUrlHint}</em>}
          </label>
          <div className={styles.row}>
            <button className={styles.btn} onClick={submit}>{t.save}</button>
            <button className={styles.btnGhost} onClick={() => { setFormOpen(false); setForm(emptyForm) }}>{t.cancel}</button>
          </div>
        </div>
      )}

      {msg && <div className={styles.msg}>{msg}</div>}
      {err && <div className={styles.err}>{err}</div>}
    </div>
  )
}
