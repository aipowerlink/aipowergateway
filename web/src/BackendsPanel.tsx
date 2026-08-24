import { useCallback, useEffect, useState } from 'react'
import styles from './BackendsPanel.module.css'
import { useT, type BackendRow } from './types'

// 内置提供方「标准配置」预设（参考 cc-switch 添加模型：选提供方即带官方 base_url + 标准模型清单）
const STANDARD_BACKENDS: Record<string, { baseUrl?: string; models: string[] }> = {
  deepseek: { baseUrl: 'https://api.deepseek.com', models: ['deepseek-chat', 'deepseek-reasoner'] },
  kimi: { baseUrl: 'https://api.moonshot.cn/v1', models: ['moonshot-v1-8k', 'moonshot-v1-32k', 'moonshot-v1-128k'] },
  zhipu: { baseUrl: 'https://open.bigmodel.cn/api/paas/v4', models: ['glm-4-flash', 'glm-4-plus'] },
  mock: { models: ['mock-7b'] },
}

interface FormState {
  editingId: string | null
  provider: string
  apiKey: string
  apiKeyEnv: string
  models: string[]
  modelInput: string
  baseUrl: string
}

const freshForm = (): FormState => ({
  editingId: null, provider: 'deepseek', apiKey: '', apiKeyEnv: '', models: [], modelInput: '', baseUrl: '',
})

// 模型设置（对齐 DeepSeek Harness settings→models；模型列表交互参考 cc-switch）
export function BackendsPanel() {
  const t = useT()
  const [rows, setRows] = useState<BackendRow[]>([])
  const [form, setForm] = useState<FormState>(freshForm)
  const [formOpen, setFormOpen] = useState(false)
  const [msg, setMsg] = useState('')
  const [err, setErr] = useState('')
  const [loading, setLoading] = useState(true)
  const [fetching, setFetching] = useState(false)
  // 连通性测试（cc-switch 式「测试」）：where 区分 表单(form) 与 卡片(row.id)
  const [test, setTest] = useState<{ where: string; busy: boolean; ok: boolean; text: string } | null>(null)

  // 测试当前表单值（不保存）
  const formTestBody = (): Record<string, unknown> => {
    const body: Record<string, unknown> = { provider: form.provider, models: form.models }
    if (form.editingId) body.id = form.editingId
    if (form.apiKey.trim()) body.apiKey = form.apiKey.trim()
    if (form.apiKeyEnv.trim()) body.apiKeyEnv = form.apiKeyEnv.trim()
    if (form.baseUrl.trim()) body.baseUrl = form.baseUrl.trim()
    return body
  }

  // 获取该提供方的具体模型列表（cc-switch「获取模型」）：用当前表单值探测，成功即填充模型 chips
  const fetchModels = async () => {
    const body = formTestBody()
    if (!body.apiKey && !body.apiKeyEnv) {
      setErr(t.apiKeyRequired)
      return
    }
    setFetching(true)
    setErr('')
    try {
      const resp = await fetch('/api/backends/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
      const data = await resp.json().catch(() => ({}))
      if (data.ok && Array.isArray(data.models) && (data.models as string[]).length > 0) {
        setForm((f) => ({ ...f, models: data.models as string[] }))
        setMsg(t.modelsFetched)
      } else {
        setErr((data.error as string) || t.fetchModelsFailed)
      }
    } catch (e) {
      setErr(String(e))
    } finally {
      setFetching(false)
    }
  }

  // silent=true：自动连接探活（只更新行状态点，不打扰全局测试指示）
  const doTest = async (body: Record<string, unknown>, where: string, silent = false) => {
    if (!silent) setTest({ where, busy: true, ok: false, text: '' })
    const markRow = (status: 'ok' | 'fail', latencyMs?: number, error?: string, models?: string[]) => {
      setRows((rs) => rs.map((r) => (r.id === where
        ? {
            ...r,
            testStatus: { status, ...(latencyMs !== undefined && { latencyMs }), ...(error ? { error } : {}) },
            // 保存后自动获取的服务器模型清单：未显式配置模型的行即时展示真实列表
            ...(models && models.length > 0 && r.models.length === 0 ? { models } : {}),
          }
        : r)))
    }
    try {
      const resp = await fetch('/api/backends/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })
      const data = await resp.json().catch(() => ({}))
      if (data.ok) {
        const lat = data.latencyMs as number | undefined
        const models = Array.isArray(data.models) ? (data.models as string[]) : undefined
        if (!silent) setTest({ where, busy: false, ok: true, text: t.testOk + (lat ? ` (${lat}ms)` : '') })
        markRow('ok', lat, undefined, models)
      } else {
        const error = (data.error as string) || String(resp.status)
        if (!silent) setTest({ where, busy: false, ok: false, text: error })
        markRow('fail', undefined, error)
      }
    } catch (e) {
      const error = String(e)
      if (!silent) setTest({ where, busy: false, ok: false, text: error })
      markRow('fail', undefined, error)
    }
  }

  // 自动连接模型服务器（DeepSeek Harness 式）：面板加载完成后对每个已配置后端静默探活，配置正确 → 绿点
  useEffect(() => {
    if (loading) return
    rows.forEach((row) => {
      doTest({ provider: row.provider, id: row.id, baseUrl: row.baseUrl || undefined }, row.id, true)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading])

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

  // 选择提供方 → 应用其标准配置（cc-switch：自定义则留空待填）
  const applyStandard = (provider: string) => {
    const std = STANDARD_BACKENDS[provider]
    if (!std) return
    setForm((f) => ({ ...f, provider, baseUrl: std.baseUrl ?? f.baseUrl, models: [...std.models] }))
  }

  const onProviderChange = (value: string) => {
    setForm((f) => {
      const std = STANDARD_BACKENDS[value]
      return std
        ? { ...f, provider: value, baseUrl: std.baseUrl ?? '', models: [...std.models], modelInput: '' }
        : { ...f, provider: value, baseUrl: '', models: [], modelInput: '' }
    })
  }

  const addModel = () => {
    const m = form.modelInput.trim()
    if (!m) return
    setForm((f) => ({
      ...f,
      models: f.models.includes(m) ? f.models : [...f.models, m],
      modelInput: '',
    }))
  }

  const removeModel = (m: string) => {
    setForm((f) => ({ ...f, models: f.models.filter((x) => x !== m) }))
  }

  const startAdd = (custom: boolean) => {
    const f = freshForm()
    f.provider = custom ? 'custom' : 'deepseek'
    if (!custom) applyStandard('deepseek')
    else setForm(f)
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
      models: [...row.models],
      modelInput: '',
      baseUrl: row.baseUrl,
    })
    setFormOpen(true)
    setMsg('')
    setErr('')
  }

  const submit = async () => {
    if (form.provider === 'custom' && (!form.baseUrl.trim() || form.models.length === 0)) {
      setErr(t.invalidCustom)
      return
    }
    const body: Record<string, unknown> = { provider: form.provider, models: form.models }
    if (form.editingId) body.id = form.editingId
    if (form.apiKey.trim()) body.apiKey = form.apiKey.trim()
    if (form.apiKeyEnv.trim()) body.apiKeyEnv = form.apiKeyEnv.trim()
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
        const savedId = (data.backend?.id as string) || form.editingId || ''
        setFormOpen(false)
        setForm(freshForm())
        await load()
        // 保存后自动连接测试（后端也已异步探活，这里让状态点即时更新）
        if (savedId) doTest({ provider: String(data.backend?.provider || form.provider), id: savedId }, savedId, true)
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
            {(() => {
              const st = row.testStatus?.status
              const title = st === 'ok'
                ? t.testOk + (row.testStatus?.latencyMs ? ` (${row.testStatus.latencyMs}ms)` : '')
                : st === 'fail' ? row.testStatus?.error || t.test : t.stateUntested
              return (
                <span className={`${styles.statusDot} ${st === 'ok' ? styles.dotOk : st === 'fail' ? styles.dotFail : styles.dotIdle}`} title={title} />
              )
            })()}
            <span className={styles.providerName}>{row.provider}</span>
            <span className={row.keySource === 'none' ? styles.badgeMissing : styles.badge}>{keyLabel(row)}</span>
            <div className={styles.chips}>
              {row.models.map((m) => <span className={styles.modelChip} key={m}>{m}</span>)}
            </div>
            <div className={styles.cardActions}>
              <button className={styles.btn} disabled={test?.busy}
                onClick={() => doTest({ provider: row.provider, id: row.id, baseUrl: row.baseUrl || undefined }, row.id)}>
                {test?.busy && test.where === row.id ? t.testing : t.test}
              </button>
              <button className={styles.btn} onClick={() => startEdit(row)}>{t.edit}</button>
              <button className={styles.btnDanger} onClick={() => del(row)}>{t.delete}</button>
            </div>
          </div>
          {row.baseUrl && <div className={styles.url}>{row.baseUrl}</div>}
          {test && test.where === row.id && (
            <div className={test.busy ? styles.testRun : test.ok ? styles.testOk : styles.testErr}>{test.text}</div>
          )}
        </div>
      ))}

      {formOpen && (
        <div className={styles.card}>
          <h3 className={styles.formTitle}>{form.editingId ? t.edit + ' — ' + form.provider : t.addProvider}</h3>
          {!form.editingId && (
            <label className={styles.field}>
              <span>{t.provider}</span>
              <select className={styles.input} value={form.provider} onChange={(e) => onProviderChange(e.target.value)}>
                <option value="deepseek">DeepSeek</option>
                <option value="kimi">Kimi</option>
                <option value="zhipu">Zhipu</option>
                <option value="mock">Mock（本地验证）</option>
                <option value="custom">{t.providerCustom}</option>
              </select>
            </label>
          )}
          {!form.editingId && form.provider !== 'custom' && (
            <>
              <button className={styles.stdBtn} onClick={() => applyStandard(form.provider)}>{t.standardModels}</button>
              <button className={styles.stdBtn} onClick={fetchModels} disabled={fetching}>
                {fetching ? t.testing : t.fetchModels}
              </button>
            </>
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
          <div className={styles.field}>
            <span>{t.modelLabel}</span>
            <div className={styles.chips}>
              {form.models.map((m) => (
                <span className={styles.modelChip} key={m}>
                  {m}
                  <button className={styles.chipRemove} onClick={() => removeModel(m)} aria-label={t.delete}>×</button>
                </span>
              ))}
            </div>
            <div className={styles.modelRow}>
              <input className={styles.input} value={form.modelInput} placeholder={t.modelPlaceholder}
                onChange={(e) => setForm({ ...form, modelInput: e.target.value })}
                onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addModel() } }} />
              <button className={styles.btn} onClick={addModel}>{t.addModel}</button>
            </div>
          </div>
          <label className={styles.field}>
            <span>{t.baseUrlLabel}</span>
            <input className={styles.input} value={form.baseUrl} placeholder="https://api.deepseek.com"
              onChange={(e) => setForm({ ...form, baseUrl: e.target.value })} />
            <em className={styles.hintSmall}>{t.customUrlHint}</em>
          </label>
          <div className={styles.row}>
            <button className={styles.btn} disabled={test?.busy} onClick={() => doTest(formTestBody(), 'form')}>
              {test?.busy && test.where === 'form' ? t.testing : t.test}
            </button>
            <button className={styles.btn} onClick={submit}>{t.save}</button>
            <button className={styles.btnGhost} onClick={() => { setFormOpen(false); setForm(freshForm()) }}>{t.cancel}</button>
          </div>
          {test && test.where === 'form' && (
            <div className={test.busy ? styles.testRun : test.ok ? styles.testOk : styles.testErr}>{test.text}</div>
          )}
        </div>
      )}

      {msg && <div className={styles.msg}>{msg}</div>}
      {err && <div className={styles.err}>{err}</div>}
    </div>
  )
}
