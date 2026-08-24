// 共享类型
export interface Usage {
  promptTokens?: number
  completionTokens?: number
  totalTokens?: number
  calls?: number
  /** 按模型累计 tokens（model -> tokens） */
  modelTokens?: Record<string, number>
}

export interface Member {
  memberId: string
  machineName: string
  ip: string
  gatewayId?: string
  banned?: boolean
  displayName: string
  online: boolean
  joinedAt: number
  lastSeen: number
  usage?: Usage
}

export type View = 'members' | 'usage' | 'controls' | 'details' | 'models'

export interface BackendRow {
  id: string
  provider: string
  /** 标准模型列表（参考 cc-switch：一提供方多模型） */
  models: string[]
  baseUrl: string
  keySource: 'file' | 'env' | 'none'
  maskedKey: string
  registered: boolean
  /** 连接测试状态（DeepSeek Harness 式状态点：ok=绿 / fail=红 / untested=灰） */
  testStatus?: { status: 'ok' | 'fail' | 'untested'; latencyMs?: number; error?: string }
}

// 中英双语字典（对应 DSH locale 模式）
export const L = {
  zh: {
    appName: 'AIPowerLink 管理面板',
    navMembers: '成员',
    navUsage: '用量',
    navControls: '控制',
    sharingOn: '共享中',
    sharingOff: '已暂停',
    startSharing: '开启共享',
    pauseSharing: '暂停共享',
    memberList: '成员列表',
    machineName: '机器名',
    displayName: '显示名',
    ip: 'IP',
    status: '状态',
    online: '在线',
    offline: '离线',
    usage: '用量',
    totalTokens: '总 token',
    calls: '调用次数',
    usageTable: '用量统计',
    controls: '管理操作',
    kick: '拉黑',
    unban: '解禁',
    banned: '已拉黑',
    gateway: '网关 ID',
    details: '成员详情',
    joinedAt: '接入时间',
    lastSeen: '最后活跃',
    back: '返回',
    noSelection: '选中成员查看详情',
    refresh: '刷新',
    actions: '操作',
    exportCsv: '导出账单 CSV',
    quota: '配额',
    unlimited: '不限',
    models: '模型分布',
    navModels: '模型',
    modelsTitle: '模型设置',
    modelsHint: '填入各提供方的 API 密钥即可使用其模型',
    provider: '提供方',
    providerCustom: '自定义提供方',
    apiKeyConfigured: 'API 密钥已配置',
    apiKeyMissing: '未配置密钥',
    addProvider: '添加提供方',
    addCustomProvider: '添加自定义提供方',
    providerName: '提供方名称',
    apiKeyLabel: 'API 密钥',
    apiKeyEnvLabel: '环境变量名',
    modelLabel: '模型',
    baseUrlLabel: 'API 地址',
    save: '保存',
    saved: '已保存',
    delete: '删除',
    cancel: '取消',
    edit: '编辑',
    keySourceFile: '密钥已保存',
    keySourceEnv: '环境变量',
    keySourceNone: '未设置',
    loading: '加载中…',
    emptyBackends: '暂无后端，点击上方按钮添加提供方',
    invalidCustom: '自定义提供方需要填写 API 地址和至少一个模型',
    customUrlHint: '留空使用官方默认地址',
    addModel: '添加模型',
    standardModels: '使用标准模型',
    modelPlaceholder: '输入模型名后回车',
    test: '测试',
    testing: '测试中…',
    testOk: '连接成功',
    stateUntested: '未测试',
    fetchModels: '获取模型',
    apiKeyRequired: '请先填写 API 密钥',
    modelsFetched: '已获取模型列表',
    fetchModelsFailed: '获取模型失败',
  },
  en: {
    appName: 'AIPowerLink Console',
    navMembers: 'Members',
    navUsage: 'Usage',
    navControls: 'Controls',
    sharingOn: 'Sharing',
    sharingOff: 'Paused',
    startSharing: 'Start sharing',
    pauseSharing: 'Pause sharing',
    memberList: 'Members',
    machineName: 'Machine',
    displayName: 'Display name',
    ip: 'IP',
    status: 'Status',
    online: 'Online',
    offline: 'Offline',
    usage: 'Usage',
    totalTokens: 'Total tokens',
    calls: 'Calls',
    usageTable: 'Usage stats',
    controls: 'Controls',
    kick: 'Ban',
    unban: 'Unban',
    banned: 'Banned',
    gateway: 'Gateway ID',
    details: 'Member details',
    joinedAt: 'Joined',
    lastSeen: 'Last seen',
    back: 'Back',
    noSelection: 'Select a member for details',
    refresh: 'Refresh',
    actions: 'Actions',
    exportCsv: 'Export CSV',
    quota: 'Quota',
    unlimited: 'Unlimited',
    models: 'Model breakdown',
    navModels: 'Models',
    modelsTitle: 'Model Settings',
    modelsHint: 'Fill in each provider API key to use its models',
    provider: 'Provider',
    providerCustom: 'Custom provider',
    apiKeyConfigured: 'API key configured',
    apiKeyMissing: 'No API key',
    addProvider: 'Add provider',
    addCustomProvider: 'Add custom provider',
    providerName: 'Provider name',
    apiKeyLabel: 'API key',
    apiKeyEnvLabel: 'Env var name',
    modelLabel: 'Model',
    baseUrlLabel: 'Base URL',
    save: 'Save',
    saved: 'Saved',
    delete: 'Delete',
    cancel: 'Cancel',
    edit: 'Edit',
    keySourceFile: 'Key saved',
    keySourceEnv: 'Env var',
    keySourceNone: 'Not set',
    loading: 'Loading…',
    emptyBackends: 'No backends yet. Add a provider above to start.',
    invalidCustom: 'Custom provider requires a base URL and at least one model',
    customUrlHint: 'Leave empty for the official default',
    addModel: 'Add model',
    standardModels: 'Use standard models',
    modelPlaceholder: 'Type a model name, press Enter',
    test: 'Test',
    testing: 'Testing…',
    testOk: 'Connected',
    stateUntested: 'Not tested',
    fetchModels: 'Fetch models',
    apiKeyRequired: 'API key required',
    modelsFetched: 'Models fetched',
    fetchModelsFailed: 'Failed to fetch models',
  },
} as const

// 语言上下文（默认英文——全球用户基线，中文可切换）
import { createContext, useContext } from 'react'
export const LangContext = createContext<{ lang: Lang; setLang: (l: Lang) => void }>({ lang: 'en', setLang: () => {} })
export function useLang() { return useContext(LangContext) }
export function useT() { const { lang } = useLang(); return L[lang] }

export type Lang = keyof typeof L
export type Dict = typeof L['zh']