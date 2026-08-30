<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Activity,
  Building2,
  Cable,
  CircleHelp,
  Gauge,
  Globe2,
  Network,
  Power,
  RefreshCw,
  Server,
  Settings,
  ShieldCheck,
  LogOut,
  WifiOff,
} from '@lucide/vue'

import AuthFlow from './components/AuthFlow.vue'
import {
  authApi,
  helperApi,
  type AuthConfiguration,
  type AuthSnapshot,
  type HelperMode,
  type HelperSnapshot,
} from './api'

const mode = ref<HelperMode>('system_split')
const snapshot = ref<HelperSnapshot | null>(null)
const busy = ref(false)
const auth = ref<AuthSnapshot | null>(null)
const authBusy = ref(false)
const authError = ref<string | null>(null)
const operationError = ref<string | null>(null)
const selectedNodeId = ref<number | null>(null)
let refreshTimer: number | undefined
let authPollTimer: number | undefined
let authPolling = false

const isRunning = computed(() => snapshot.value?.state === 'running')
const canConnect = computed(
  () => Boolean(auth.value?.authenticated && selectedNodeId.value) && !busy.value && !isRunning.value,
)
const statusLabel = computed(() => {
  if (!snapshot.value?.reachable) return '服务未启动'
  const labels = {
    idle: '已断开',
    starting: '正在连接',
    running: '已连接',
    stopping: '正在断开',
    failed: '需要清理',
  }
  return labels[snapshot.value.state]
})

const statusDetail = computed(() => {
  if (snapshot.value?.active) {
    return `${snapshot.value.active.node_name} · ${snapshot.value.active.address}`
  }
  return snapshot.value?.error?.message ?? '选择企业节点后即可建立安全连接'
})

const selectedNode = computed(() =>
  auth.value?.nodes.find((node) => node.id === selectedNodeId.value),
)

function errorMessage(error: unknown) {
  if (typeof error === 'object' && error && 'message' in error) {
    const message = String(error.message)
    return 'code' in error ? `[${String(error.code)}] ${message}` : message
  }
  return String(error)
}

function applyAuth(next: AuthSnapshot) {
  auth.value = next
  if (!selectedNodeId.value && next.nodes.length > 0) {
    selectedNodeId.value = next.nodes[0].id
  }
  if (next.authenticated || !next.challenge) {
    window.clearInterval(authPollTimer)
    authPollTimer = undefined
  } else if (!authPollTimer) {
    authPollTimer = window.setInterval(pollQr, 2000)
  }
}

async function loadAuth() {
  try {
    applyAuth(await authApi.status())
  } catch {
    applyAuth({
      configured: false,
      authenticated: false,
      company_code: null,
      company_name: null,
      platform: null,
      challenge: null,
      nodes: [],
    })
  }
}

async function configureAuth(configuration: AuthConfiguration) {
  authBusy.value = true
  authError.value = null
  try {
    applyAuth(await authApi.configure(configuration))
    await beginQr()
  } catch (error) {
    authError.value = errorMessage(error)
  } finally {
    authBusy.value = false
  }
}

async function beginQr() {
  authBusy.value = true
  authError.value = null
  try {
    applyAuth(await authApi.beginQr())
  } catch (error) {
    authError.value = errorMessage(error)
  } finally {
    authBusy.value = false
  }
}

async function pollQr() {
  if (authPolling) return
  authPolling = true
  try {
    applyAuth(await authApi.pollQr())
    authError.value = null
  } catch (error) {
    authError.value = errorMessage(error)
    try {
      applyAuth(await authApi.status())
    } catch {
      // Keep the active challenge polling; a later attempt may recover.
    }
  } finally {
    authPolling = false
  }
}

async function resetAuth() {
  authBusy.value = true
  authError.value = null
  try {
    applyAuth(await authApi.reset())
    selectedNodeId.value = null
  } catch (error) {
    authError.value = errorMessage(error)
  } finally {
    authBusy.value = false
  }
}

async function refreshNodes() {
  if (!auth.value?.authenticated) return
  authBusy.value = true
  try {
    applyAuth(await authApi.refreshNodes())
  } catch (error) {
    authError.value = errorMessage(error)
  } finally {
    authBusy.value = false
  }
}

async function refresh() {
  try {
    snapshot.value = await helperApi.status(mode.value)
  } catch {
    snapshot.value = {
      mode: mode.value,
      reachable: false,
      state: 'idle',
      active: null,
      stats: { tx_bytes: 0, rx_bytes: 0 },
      error: {
        code: 'desktop_bridge_unavailable',
        message: '桌面控制器尚未就绪',
        retryable: true,
      },
    }
  }
}

async function changeMode(nextMode: HelperMode) {
  if (isRunning.value) return
  mode.value = nextMode
  await refresh()
}

async function disconnect() {
  busy.value = true
  operationError.value = null
  try {
    snapshot.value = await helperApi.stop(mode.value)
  } catch (error) {
    operationError.value = errorMessage(error)
  } finally {
    busy.value = false
  }
}

async function cleanup() {
  busy.value = true
  operationError.value = null
  try {
    snapshot.value = await helperApi.cleanup(mode.value)
  } catch (error) {
    operationError.value = errorMessage(error)
  } finally {
    busy.value = false
  }
}

async function connect() {
  if (!selectedNodeId.value) return
  busy.value = true
  operationError.value = null
  try {
    snapshot.value = await helperApi.connect(mode.value, selectedNodeId.value)
  } catch (error) {
    operationError.value = errorMessage(error)
  } finally {
    busy.value = false
  }
}

function formatBytes(bytes = 0) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 ** 2).toFixed(1)} MB`
}

onMounted(async () => {
  await Promise.all([refresh(), loadAuth()])
  refreshTimer = window.setInterval(refresh, 5000)
})

onUnmounted(() => {
  window.clearInterval(refreshTimer)
  window.clearInterval(authPollTimer)
})
</script>

<template>
  <main class="app-shell">
    <aside class="sidebar">
      <div class="brand" aria-label="Feilian Lite">
        <span class="brand-mark"><ShieldCheck :size="21" /></span>
        <div>
          <strong>Feilian Lite</strong>
          <span>企业网络</span>
        </div>
      </div>

      <nav aria-label="主导航">
        <button class="nav-item active" type="button"><Gauge :size="18" />连接</button>
        <button class="nav-item" type="button"><Activity :size="18" />诊断</button>
        <button class="nav-item" type="button"><Settings :size="18" />设置</button>
      </nav>

      <div class="sidebar-foot">
        <span class="health-dot" :class="{ online: snapshot?.reachable }"></span>
        <span>{{ snapshot?.reachable ? 'Helper 在线' : 'Helper 离线' }}</span>
        <span class="version">0.1.0</span>
      </div>
    </aside>

    <section class="workspace">
      <header class="topbar">
        <div>
          <p class="eyebrow">CONNECTION</p>
          <h1>连接控制台</h1>
        </div>
        <div class="top-actions">
          <div v-if="auth?.authenticated" class="identity-pill">
            <Building2 :size="15" />
            <span>{{ auth.company_name }}</span>
            <button type="button" title="退出企业" @click="resetAuth"><LogOut :size="14" /></button>
          </div>
          <button class="icon-button" type="button" title="刷新状态" :disabled="busy" @click="refresh">
            <RefreshCw :size="18" />
          </button>
        </div>
      </header>

      <section class="connection-band" :class="{ connected: isRunning }">
        <div class="status-copy">
          <span class="status-icon">
            <Cable v-if="isRunning" :size="28" />
            <WifiOff v-else :size="28" />
          </span>
          <div>
            <p class="status-label">{{ statusLabel }}</p>
            <p class="status-detail">{{ statusDetail }}</p>
          </div>
        </div>
        <button
          v-if="isRunning"
          class="power-button danger"
          type="button"
          :disabled="busy"
          @click="disconnect"
        >
          <Power :size="19" />断开连接
        </button>
        <button v-else class="power-button" type="button" :disabled="!canConnect" @click="connect">
          <Power :size="19" />{{ busy ? '正在连接' : selectedNodeId ? '连接' : '选择节点' }}
        </button>
      </section>

      <section class="control-grid">
        <div class="control-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">TUNNEL MODE</p>
              <h2>连接模式</h2>
            </div>
            <CircleHelp :size="17" />
          </div>

          <div class="segmented" role="tablist" aria-label="连接模式">
            <button
              type="button"
              :disabled="isRunning"
              :class="{ selected: mode === 'system_split' }"
              @click="changeMode('system_split')"
            >
              <Network :size="18" />
              <span><strong>系统分流</strong><small>企业流量进入隧道</small></span>
            </button>
            <button
              type="button"
              :disabled="isRunning"
              :class="{ selected: mode === 'socks5' }"
              @click="changeMode('socks5')"
            >
              <Globe2 :size="18" />
              <span><strong>SOCKS5</strong><small>仅代理指定应用</small></span>
            </button>
          </div>

          <div class="node-row">
            <Server :size="18" />
            <label>
              <span>当前节点</span>
              <select v-model="selectedNodeId" :disabled="!auth?.authenticated" @focus="refreshNodes">
                <option :value="null">{{ auth?.authenticated ? '选择节点' : '登录后可用' }}</option>
                <option v-for="node in auth?.nodes" :key="node.id" :value="node.id">
                  {{ node.name }} · {{ node.protocol.toUpperCase() }}
                </option>
              </select>
            </label>
            <span class="latency">{{ selectedNode?.protocol?.toUpperCase() ?? '--' }}</span>
          </div>
        </div>

        <div class="metrics-panel">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">SESSION</p>
              <h2>实时流量</h2>
            </div>
            <Activity :size="17" />
          </div>
          <div class="metrics">
            <div><span>下载</span><strong>{{ formatBytes(snapshot?.stats.rx_bytes) }}</strong></div>
            <div><span>上传</span><strong>{{ formatBytes(snapshot?.stats.tx_bytes) }}</strong></div>
            <div><span>协议</span><strong>{{ snapshot?.active?.protocol?.toUpperCase() ?? '--' }}</strong></div>
          </div>
        </div>
      </section>

      <section v-if="operationError || snapshot?.error" class="notice-band">
        <div>
          <strong>{{ operationError ? '连接操作失败' : snapshot?.error?.code === 'helper_unavailable' ? 'Tunnel Helper 尚未运行' : '连接组件异常' }}</strong>
          <span>{{ operationError ?? snapshot?.error?.message }}</span>
        </div>
        <button v-if="!operationError && snapshot?.error?.retryable" type="button" :disabled="busy" @click="cleanup">
          <RefreshCw :size="16" />尝试清理
        </button>
      </section>
    </section>

    <AuthFlow
      v-if="auth && !auth.authenticated"
      :snapshot="auth"
      :busy="authBusy"
      :error="authError"
      @configure="configureAuth"
      @begin-qr="beginQr"
      @reset="resetAuth"
    />
  </main>
</template>
