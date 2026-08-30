import { expect, test } from '@playwright/test'

test('browser preview reports desktop requirement without leaking a Tauri TypeError', async ({
  page,
}) => {
  await page.goto('/')
  await page.getByLabel('企业代码').fill('acme')
  await page.getByRole('button', { name: '验证企业' }).click()

  await expect(page.getByText('请在 Feilian Lite 桌面应用中完成企业验证')).toBeVisible()
  await expect(page.getByText(/Cannot read properties of undefined/)).toHaveCount(0)
})

test('QR polling recovers after a transient backend error', async ({ page }) => {
  await page.addInitScript(() => {
    const pendingAuth = {
      configured: true,
      authenticated: false,
      company_code: 'acme',
      company_name: 'Acme Technology',
      platform: 'feishu',
      challenge: {
        login_url: 'https://passport.example.test/qr/desktop',
        expires_at_unix: null,
      },
      nodes: [],
    }
    let pollCount = 0

    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          if (command === 'auth_status') return pendingAuth
          if (command === 'auth_poll_qr') {
            pollCount += 1
            if (pollCount === 1) throw new Error('temporary network failure')
            return {
              ...pendingAuth,
              authenticated: true,
              challenge: null,
              nodes: [{ id: 1, name: '上海节点', english_name: 'Shanghai-01', address: '192.0.2.1:51820', protocol: 'udp', latency_ms: 36, available: true }],
            }
          }
          if (command.startsWith('helper_')) {
            return {
              mode: 'system_split',
              reachable: false,
              state: 'idle',
              active: null,
              stats: { tx_bytes: 0, rx_bytes: 0 },
              error: null,
            }
          }
          throw new Error(`Unexpected command: ${command}`)
        },
      },
    })
  })

  await page.goto('/')
  await expect(page.getByText('temporary network failure')).toBeVisible({ timeout: 4_000 })
  await expect(page.getByRole('heading', { name: '等待飞书确认' })).toBeHidden({ timeout: 5_000 })
  await expect(page.getByLabel('选择 VPN 节点')).toContainText('上海节点')
})

test('diagnostics and settings navigation open functional views', async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 560 })
  await page.addInitScript(() => {
    const auth = {
      configured: true,
      authenticated: true,
      company_code: 'acme',
      company_name: 'Acme Technology',
      platform: 'feishu',
      challenge: null,
      nodes: [],
    }
    const helper = {
      mode: 'system_split',
      reachable: true,
      state: 'idle',
      active: null,
      stats: { tx_bytes: 1024, rx_bytes: 2048 },
      error: null,
    }
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          if (command.startsWith('auth_')) return auth
          if (command.startsWith('helper_')) return helper
          throw new Error(`Unexpected command: ${command}`)
        },
      },
    })
  })

  await page.goto('/')
  await page.getByRole('button', { name: '诊断' }).click()
  await expect(page.getByRole('heading', { name: '运行诊断' })).toBeVisible()
  await expect(page.getByText('Helper 通信')).toBeVisible()
  await page.screenshot({ path: 'test-results/dashboard-diagnostics.png', fullPage: true })

  await page.getByRole('button', { name: '设置' }).click()
  await expect(page.getByRole('heading', { name: '客户端设置' })).toBeVisible()
  await expect(page.getByText('系统凭据库')).toBeVisible()
  await page.getByRole('button', { name: /SOCKS5/ }).click()
  await expect(page.getByRole('button', { name: /SOCKS5/ })).toHaveClass(/selected/)
  await page.reload()
  await page.getByRole('button', { name: '设置' }).click()
  await expect(page.getByRole('button', { name: /SOCKS5/ })).toHaveClass(/selected/)
  await page.screenshot({ path: 'test-results/dashboard-settings.png', fullPage: true })

  const hasHorizontalOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  )
  expect(hasHorizontalOverflow).toBe(false)

  await page.getByRole('button', { name: '连接' }).click()
  await expect(page.getByRole('heading', { name: '连接控制台' })).toBeVisible()
})

const scenarios = [
  { name: 'configure', width: 1080, height: 720, auth: 'configure' },
  { name: 'qr', width: 900, height: 680, auth: 'qr' },
  { name: 'authenticated-compact', width: 760, height: 560, auth: 'authenticated' },
] as const

for (const scenario of scenarios) {
  test(`desktop workflow fits ${scenario.name}`, async ({ page }) => {
    await page.setViewportSize(scenario)
    await page.addInitScript((authState) => {
      const helper = {
        mode: 'system_split',
        reachable: false,
        state: 'idle',
        active: null,
        stats: { tx_bytes: 0, rx_bytes: 0 },
        error: { code: 'helper_unavailable', message: 'Tunnel helper is not running', retryable: true },
      }
      const auth = {
        configured: authState !== 'configure',
        authenticated: authState === 'authenticated',
        company_code: authState === 'configure' ? null : 'acme',
        company_name: authState === 'configure' ? null : 'Acme Technology',
        platform: authState === 'configure' ? null : 'feishu',
        challenge:
          authState === 'qr'
            ? { login_url: 'https://passport.example.test/qr/desktop', expires_at_unix: null }
            : null,
        nodes:
          authState === 'authenticated'
            ? [
                { id: 1, name: '上海节点', english_name: 'Shanghai-01', address: '192.0.2.1:51820', protocol: 'udp', latency_ms: 36, available: true },
                { id: 2, name: '北京节点', english_name: 'Beijing-02', address: '192.0.2.2:443', protocol: 'tcp', latency_ms: 128, available: true },
                { id: 3, name: '备用节点', english_name: null, address: '192.0.2.3:51820', protocol: 'udp', latency_ms: null, available: false },
              ]
            : [],
      }

      Object.assign(window, {
        __COMMANDS__: [] as Array<{ command: string; args: unknown }>,
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            ;(window as typeof window & { __COMMANDS__: Array<{ command: string; args: unknown }> })
              .__COMMANDS__.push({ command, args })
            if (command === 'helper_connect') {
              await new Promise((resolve) => setTimeout(resolve, 150))
              return {
                ...helper,
                reachable: true,
                state: 'running',
                active: {
                  node_name: '上海节点',
                  interface_name: 'feilian-lite',
                  mode: 'system_split',
                  address: '10.0.0.2/32',
                  protocol: 'udp',
                },
                error: null,
              }
            }
            if (command.startsWith('helper_')) return helper
            if (command.startsWith('auth_')) return auth
            throw new Error(`Unexpected command: ${command}`)
          },
        },
      })
    }, scenario.auth)
    await page.goto('/')

    await expect(page.getByRole('heading', { name: '连接控制台' })).toBeVisible()
    if (scenario.auth === 'configure') {
      await expect(page.getByRole('heading', { name: '连接企业空间' })).toBeVisible()
      await expect(page.getByLabel('企业代码')).toBeVisible()
    } else if (scenario.auth === 'qr') {
      await expect(page.getByText('等待飞书确认')).toBeVisible()
      await expect(page.locator('.qr-frame > svg')).toBeVisible()
    } else {
      await expect(page.getByText('Acme Technology')).toBeVisible()
      const nodePicker = page.getByLabel('选择 VPN 节点')
      await expect(nodePicker).toContainText('上海节点')
      await expect(nodePicker).toContainText('Shanghai-01')
      await expect(nodePicker).toContainText('192.0.2.1:51820')
      await expect(nodePicker).toContainText('36 ms')
      await nodePicker.click()
      const beijingNode = page.getByRole('option', { name: /北京节点.*Beijing-02.*192\.0\.2\.2:443.*128 ms/ })
      await expect(beijingNode).toBeVisible()
      await expect(page.getByRole('option', { name: /备用节点.*不可达/ })).toBeDisabled()
      await page.screenshot({ path: 'test-results/dashboard-node-picker-open.png', fullPage: true })
      await beijingNode.click()
      await expect(nodePicker).toContainText('北京节点')
      await expect(nodePicker).toContainText('128 ms')
      await page.locator('.power-button').getByText('连接', { exact: true }).click()
      await expect(page.getByText('等待管理员授权')).toBeVisible()
      await expect(page.getByRole('button', { name: '断开连接' })).toBeVisible()
      await expect(page.getByRole('button', { name: /系统分流/ })).toBeDisabled()
      await expect(page.getByRole('button', { name: /SOCKS5/ })).toBeDisabled()
      const connectCall = await page.evaluate(() =>
        (
          window as typeof window & {
            __COMMANDS__: Array<{ command: string; args: { nodeId?: number } }>
          }
        ).__COMMANDS__.find((call) => call.command === 'helper_connect'),
      )
      expect(connectCall?.args.nodeId).toBe(2)
    }

    const hasHorizontalOverflow = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
    )
    expect(hasHorizontalOverflow).toBe(false)

    await page.screenshot({
      path: `test-results/dashboard-${scenario.name}.png`,
      fullPage: true,
    })
  })
}