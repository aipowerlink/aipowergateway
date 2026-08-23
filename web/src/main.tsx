import React from 'react'
import { createRoot } from 'react-dom/client'
import { AppFrame } from './AppFrame'
import './base.css'

// 薄壳：只挂载 #root（对应 DSH apps/web 入口）
const el = document.getElementById('root')
if (!el) throw new Error('missing #root')
createRoot(el).render(
  <React.StrictMode>
    <AppFrame />
  </React.StrictMode>,
)