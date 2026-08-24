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

export type View = 'members' | 'usage' | 'controls' | 'details'

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
  },
} as const

// 语言上下文（默认英文——全球用户基线，中文可切换）
import { createContext, useContext } from 'react'
export const LangContext = createContext<{ lang: Lang; setLang: (l: Lang) => void }>({ lang: 'en', setLang: () => {} })
export function useLang() { return useContext(LangContext) }
export function useT() { const { lang } = useLang(); return L[lang] }

export type Lang = keyof typeof L
export type Dict = typeof L['zh']