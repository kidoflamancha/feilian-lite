import { invoke } from '@tauri-apps/api/core'

type TauriWindow = Window & {
  __TAURI_INTERNALS__?: {
    invoke?: unknown
  }
}

export class DesktopRuntimeUnavailableError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'DesktopRuntimeUnavailableError'
  }
}

function hasTauriBridge() {
  return (
    typeof window !== 'undefined' &&
    typeof (window as TauriWindow).__TAURI_INTERNALS__?.invoke === 'function'
  )
}

function invokeDesktop<T>(command: string, args: Record<string, unknown>, unavailableMessage: string) {
  if (!hasTauriBridge()) {
    return Promise.reject(new DesktopRuntimeUnavailableError(unavailableMessage))
  }
  return invoke<T>(command, args)
}

export type HelperMode = 'system_split' | 'socks5'
export type HelperState = 'idle' | 'starting' | 'running' | 'stopping' | 'failed'

export interface ActiveTunnel {
  node_name: string
  interface_name: string
  mode: HelperMode
  address: string
  protocol: 'udp' | 'tcp'
}

export interface ControllerError {
  code: string
  message: string
  retryable: boolean
}

export interface HelperSnapshot {
  mode: HelperMode
  reachable: boolean
  state: HelperState
  active: ActiveTunnel | null
  stats: {
    tx_bytes: number
    rx_bytes: number
  }
  error: ControllerError | null
}

export type AuthPlatform = 'feishu' | 'oidc'

export interface AuthNode {
  id: number
  name: string
  english_name: string | null
  address: string
  protocol: 'udp' | 'tcp' | 'unknown'
  latency_ms: number | null
  available: boolean
}

export interface DesktopQrChallenge {
  login_url: string
  expires_at_unix: number | null
}

export interface AuthSnapshot {
  configured: boolean
  authenticated: boolean
  company_code: string | null
  company_name: string | null
  platform: AuthPlatform | null
  challenge: DesktopQrChallenge | null
  nodes: AuthNode[]
}

export interface AuthConfiguration {
  company_code: string
  platform: AuthPlatform
}

export const helperApi = {
  status: (mode: HelperMode) =>
    invokeDesktop<HelperSnapshot>('helper_status', { mode }, '请在 Feilian Lite 桌面应用中查看连接状态'),
  connect: (mode: HelperMode, nodeId: number) =>
    invokeDesktop<HelperSnapshot>(
      'helper_connect',
      { mode, nodeId },
      '请在 Feilian Lite 桌面应用中建立 VPN 连接',
    ),
  stop: (mode: HelperMode) =>
    invokeDesktop<HelperSnapshot>('helper_stop', { mode }, '请在 Feilian Lite 桌面应用中断开 VPN 连接'),
  cleanup: (mode: HelperMode) =>
    invokeDesktop<HelperSnapshot>('helper_cleanup', { mode }, '请在 Feilian Lite 桌面应用中清理连接'),
}

export const authApi = {
  status: () => invokeDesktop<AuthSnapshot>('auth_status', {}, '请在 Feilian Lite 桌面应用中查看登录状态'),
  configure: (configuration: AuthConfiguration) =>
    invokeDesktop<AuthSnapshot>(
      'auth_configure',
      { configuration },
      '请在 Feilian Lite 桌面应用中完成企业验证',
    ),
  beginQr: () =>
    invokeDesktop<AuthSnapshot>('auth_begin_qr', {}, '请在 Feilian Lite 桌面应用中完成扫码登录'),
  pollQr: () =>
    invokeDesktop<AuthSnapshot>('auth_poll_qr', {}, '请在 Feilian Lite 桌面应用中完成扫码登录'),
  refreshNodes: () =>
    invokeDesktop<AuthSnapshot>('auth_refresh_nodes', {}, '请在 Feilian Lite 桌面应用中刷新节点'),
  reset: () => invokeDesktop<AuthSnapshot>('auth_reset', {}, '请在 Feilian Lite 桌面应用中更换企业'),
}