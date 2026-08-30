<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Activity,
  BadgeCheck,
  Building2,
  Cable,
  ChevronDown,
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
  MapPin,
  Radio,
  WifiOff,
} from '@lucide/vue'

import AuthFlow from './components/AuthFlow.vue'
import {
  authApi,
  helperApi,
  type AuthConfiguration,
  type AuthNode,
  type AuthSnapshot,
  type HelperMode,
  type HelperSnapshot,
} from './api'

const savedMode = window.localStorage.getItem('feilian.helper-mode')
const mode = ref<HelperMode>(savedMode === 'socks5' ? 'socks5' : 'system_split')
const activeView = ref<'connection' | 'diagnostics' | 'settings'>('connection')
const snapshot = ref<HelperSnapshot | null>(null)
const busy = ref(false)
const auth = ref<AuthSnapshot | null>(null)
const authBusy = ref(false)
const authError = ref<string | null>(null)
const operationError = ref<string | null>(null)
const selectedNodeId = ref<number | null>(null)
const nodePicker = ref<HTMLDetailsElement | null>(null)
let refreshTimer: number | undefined
let authPollTimer: number | undefined
let authPolling = false

const isRunning = computed(() => snapshot.value?.state === 'running')
const selectedNode = computed(() =>
  auth.value?.nodes.find((node) => node.id === selectedNodeId.value),
)
const canConnect = computed(
  () => Boolean(auth.value?.authenticated && selectedNode.value?.available) && !busy.value && !isRunning.value,
)
const pageTitle = computed(() => {
  const pages = {
    connection: { eyebrow: 'CONNECTION', title: '连接控制台' },
    diagnostics: { eyebrow: 'DIAGNOSTICS', title: '运行诊断' },
    settings: { eyebrow: 'SETTINGS', title: '客户端设置' },
  }
  return pages[activeView.value]
})
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

function errorMessage(error: unknown) {
  if (typeof error === 'object' && error && 'message' in error) {
    const message = String(error.message)
    return 'code' in error ? `[${String(error.code)}] ${message}` : message
  }
  return String(error)
}

function applyAuth(next: AuthSnapshot) {
  auth.value = next
  if (!next.nodes.some((node) => node.id === selectedNodeId.value && node.available)) {
    selectedNodeId.value = next.nodes.find((node) => node.available)?.id ?? null
  }
  if (next.authenticated || !next.challenge) {
    window.clearInterval(authPollTimer)
    authPollTimer = undefined
  } else if (!authPollTimer) {
    authPollTimer = window.setInterval(pollQr, 2000)
  }
}

function latencyLabel(node: AuthNode) {
  return node.latency_ms === null ? '不可达' : `${node.latency_ms} ms`
}

function latencyClass(node: AuthNode) {
  if (node.latency_ms === null) return 'unavailable'
  if (node.latency_ms < 80) return 'fast'
  if (node.latency_ms < 180) return 'medium'
  return 'slow'
}

function selectNode(node: AuthNode) {
  if (!node.available || isRunning.value) return
  selectedNodeId.value = node.id
  if (nodePicker.value) nodePicker.value.open = false
}

function handleNodePickerToggle(event: Event) {
  const details = event.currentTarget as HTMLDetailsElement
  if (details.open && auth.value?.authenticated && !authBusy.value) void refreshNodes()
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

async function refreshAll() {
  await Promise.all([refresh(), refreshNodes()])
}

async function changeMode(nextMode: HelperMode) {
  if (isRunning.value) return
  mode.value = nextMode
  window.localStorage.setItem('feilian.helper-mode', nextMode)
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
        <button class="nav-item" :class="{ active: activeView === 'connection' }" :aria-current="activeView === 'connection' ? 'page' : undefined" type="button" @click="activeView = 'connection'"><Gauge :size="18" />连接</button>
        <button class="nav-item" :class="{ active: activeView === 'diagnostics' }" :aria-current="activeView === 'diagnostics' ? 'page' : undefined" type="button" @click="activeView = 'diagnostics'"><Activity :size="18" />诊断</button>
        <button class="nav-item" :class="{ active: activeView === 'settings' }" :aria-current="activeView === 'settings' ? 'page' : undefined" type="button" @click="activeView = 'settings'"><Settings :size="18" />设置</button>
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
          <p class="eyebrow">{{ pageTitle.eyebrow }}</p>
          <h1>{{ pageTitle.title }}</h1>
        </div>
        <div class="top-actions">
          <div v-if="auth?.authenticated" class="identity-pill">
            <Building2 :size="15" />
            <span>{{ auth.company_name }}</span>
            <button type="button" title="退出企业" @click="resetAuth"><LogOut :size="14" /></button>
          </div>
          <button class="icon-button" type="button" title="刷新状态" :disabled="busy || authBusy" @click="refreshAll">
            <RefreshCw :size="18" />
          </button>
        </div>
      </header>

      <template v-if="activeView === 'connection'">
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
              <details
                ref="nodePicker"
                class="node-picker"
                :class="{ disabled: !auth?.authenticated || isRunning }"
                @toggle="handleNodePickerToggle"
              >
                <summary
                  aria-label="选择 VPN 节点"
                  @click="(!auth?.authenticated || isRunning) && $event.preventDefault()"
                >
                  <span class="selected-node-copy">
                    <strong>{{ selectedNode?.name ?? (auth?.authenticated ? '选择节点' : '登录后可用') }}</strong>
                    <small v-if="selectedNode">
                      {{ selectedNode.english_name ? `${selectedNode.english_name} · ` : '' }}{{ selectedNode.address }}
                    </small>
                  </span>
                  <span v-if="selectedNode" class="node-summary-status">
                    <b>{{ selectedNode.protocol.toUpperCase() }}</b>
                    <em :class="latencyClass(selectedNode)">{{ latencyLabel(selectedNode) }}</em>
                  </span>
                  <RefreshCw v-if="authBusy" class="spin" :size="14" />
                  <ChevronDown v-else class="node-chevron" :size="16" />
                </summary>
                <div class="node-menu" role="listbox" aria-label="VPN 节点列表">
                  <button
                    v-for="node in auth?.nodes"
                    :key="node.id"
                    type="button"
                    role="option"
                    :aria-selected="node.id === selectedNodeId"
                    :disabled="!node.available || isRunning"
                    @click="selectNode(node)"
                  >
                    <span class="node-identity">
                      <strong>{{ node.name }}</strong>
                      <small v-if="node.english_name">{{ node.english_name }}</small>
                      <span><MapPin :size="12" />{{ node.address }}</span>
                    </span>
                    <span class="node-option-status">
                      <b>{{ node.protocol.toUpperCase() }}</b>
                      <em :class="latencyClass(node)">{{ latencyLabel(node) }}</em>
                    </span>
                  </button>
                  <p v-if="!auth?.nodes.length">暂无可用节点</p>
                </div>
              </details>
            </label>
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
      </template>

      <template v-else-if="activeView === 'diagnostics'">
        <section class="diagnostics-grid">
          <div class="diagnostic-panel">
            <div class="panel-heading">
              <div><p class="eyebrow">HEALTH</p><h2>组件状态</h2></div>
              <Activity :size="17" />
            </div>
            <div class="status-list">
              <div>
                <span><Radio :size="15" />Helper 通信</span>
                <strong :class="snapshot?.reachable ? 'healthy' : 'warning'">{{ snapshot?.reachable ? '正常' : '不可用' }}</strong>
              </div>
              <div>
                <span><BadgeCheck :size="15" />认证会话</span>
                <strong :class="auth?.authenticated ? 'healthy' : 'warning'">{{ auth?.authenticated ? '已认证' : '未认证' }}</strong>
              </div>
              <div>
                <span><Cable :size="15" />隧道状态</span>
                <strong>{{ statusLabel }}</strong>
              </div>
              <div>
                <span><Server :size="15" />可用节点</span>
                <strong>{{ auth?.nodes.filter((node) => node.available).length ?? 0 }} / {{ auth?.nodes.length ?? 0 }}</strong>
              </div>
            </div>
          </div>

          <div class="diagnostic-panel">
            <div class="panel-heading">
              <div><p class="eyebrow">SESSION</p><h2>当前会话</h2></div>
              <Gauge :size="17" />
            </div>
            <div class="status-list">
              <div><span>连接模式</span><strong>{{ mode === 'system_split' ? '系统分流' : 'SOCKS5' }}</strong></div>
              <div><span>当前节点</span><strong>{{ snapshot?.active?.node_name ?? selectedNode?.name ?? '--' }}</strong></div>
              <div><span>下载流量</span><strong>{{ formatBytes(snapshot?.stats.rx_bytes) }}</strong></div>
              <div><span>上传流量</span><strong>{{ formatBytes(snapshot?.stats.tx_bytes) }}</strong></div>
            </div>
          </div>
        </section>

        <section v-if="operationError || snapshot?.error" class="notice-band diagnostics-notice">
          <div><strong>最近错误</strong><span>{{ operationError ?? snapshot?.error?.message }}</span></div>
        </section>

        <div class="view-actions">
          <button type="button" :disabled="busy || authBusy" @click="refreshAll"><RefreshCw :size="16" />重新检测</button>
          <button type="button" :disabled="busy || isRunning" @click="cleanup"><ShieldCheck :size="16" />清理残留状态</button>
        </div>
      </template>

      <template v-else>
        <section class="settings-stack">
          <div class="settings-panel">
            <div class="panel-heading">
              <div><p class="eyebrow">CONNECTION MODE</p><h2>默认连接模式</h2></div>
              <Network :size="17" />
            </div>
            <div class="segmented" role="tablist" aria-label="设置连接模式">
              <button type="button" :disabled="isRunning" :class="{ selected: mode === 'system_split' }" @click="changeMode('system_split')">
                <Network :size="18" /><span><strong>系统分流</strong><small>企业网段进入隧道</small></span>
              </button>
              <button type="button" :disabled="isRunning" :class="{ selected: mode === 'socks5' }" @click="changeMode('socks5')">
                <Globe2 :size="18" /><span><strong>SOCKS5</strong><small>本地 127.0.0.1:1080</small></span>
              </button>
            </div>
          </div>

          <div class="settings-panel settings-row">
            <span class="setting-icon"><ShieldCheck :size="20" /></span>
            <div><strong>系统凭据库</strong><span>WireGuard 私钥与认证密钥由操作系统安全存储保护</span></div>
            <b class="setting-state">已启用</b>
          </div>

          <div class="settings-panel settings-row">
            <span class="setting-icon"><Building2 :size="20" /></span>
            <div><strong>{{ auth?.company_name ?? '未连接企业' }}</strong><span>{{ auth?.authenticated ? `认证方式：${auth.platform === 'oidc' ? 'OIDC' : '飞书'}` : '需要完成企业认证' }}</span></div>
            <button class="secondary-button danger-text" type="button" :disabled="!auth?.configured || isRunning || authBusy" @click="resetAuth"><LogOut :size="15" />退出企业</button>
          </div>
        </section>
      </template>
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
