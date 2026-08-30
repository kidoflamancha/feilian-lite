<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Building2, KeyRound, LoaderCircle, LogIn, RotateCcw, ScanLine } from '@lucide/vue'
import QrcodeVue from 'qrcode.vue'

import type { AuthConfiguration, AuthPlatform, AuthSnapshot } from '../api'

const props = defineProps<{
  snapshot: AuthSnapshot | null
  busy: boolean
  error: string | null
}>()

const emit = defineEmits<{
  configure: [configuration: AuthConfiguration]
  beginQr: []
  reset: []
}>()

const companyCode = ref('')
const platform = ref<AuthPlatform>('feishu')

watch(
  () => props.snapshot,
  (snapshot) => {
    if (snapshot?.company_code) companyCode.value = snapshot.company_code
    if (snapshot?.platform) platform.value = snapshot.platform
  },
  { immediate: true },
)

const phase = computed(() => {
  if (!props.snapshot?.configured) return 'configure'
  if (props.snapshot.challenge) return 'qr'
  return 'ready'
})

function submitConfiguration() {
  emit('configure', {
    company_code: companyCode.value.trim(),
    platform: platform.value,
  })
}
</script>

<template>
  <div class="auth-backdrop">
    <section class="auth-dialog" role="dialog" aria-modal="true" aria-labelledby="auth-title">
      <header class="auth-header">
        <span class="auth-mark"><Building2 :size="22" /></span>
        <div>
          <p class="eyebrow">ENTERPRISE ACCESS</p>
          <h2 id="auth-title">{{ phase === 'configure' ? '连接企业空间' : snapshot?.company_name }}</h2>
        </div>
      </header>

      <form v-if="phase === 'configure'" class="auth-form" @submit.prevent="submitConfiguration">
        <label>
          <span>企业代码</span>
          <input v-model="companyCode" name="company-code" autocomplete="organization" placeholder="company-code" required />
        </label>

        <fieldset>
          <legend>认证方式</legend>
          <div class="auth-methods">
            <button type="button" :class="{ selected: platform === 'feishu' }" @click="platform = 'feishu'">
              <ScanLine :size="18" /><span><strong>飞书扫码</strong><small>Feishu SSO</small></span>
            </button>
            <button type="button" :class="{ selected: platform === 'oidc' }" @click="platform = 'oidc'">
              <KeyRound :size="18" /><span><strong>OIDC</strong><small>企业身份提供方</small></span>
            </button>
          </div>
        </fieldset>

        <button class="auth-primary" type="submit" :disabled="busy || !companyCode.trim()">
          <LoaderCircle v-if="busy" class="spin" :size="18" />
          <LogIn v-else :size="18" />
          验证企业
        </button>
      </form>

      <div v-else-if="phase === 'ready'" class="auth-ready">
        <span class="provider-icon"><ScanLine :size="26" /></span>
        <strong>{{ snapshot?.platform === 'oidc' ? 'OIDC 登录' : '飞书扫码登录' }}</strong>
        <button class="auth-primary" type="button" :disabled="busy" @click="emit('beginQr')">
          <LoaderCircle v-if="busy" class="spin" :size="18" />
          <ScanLine v-else :size="18" />
          获取二维码
        </button>
        <button class="auth-link" type="button" :disabled="busy" @click="emit('reset')">更换企业</button>
      </div>

      <div v-else class="qr-stage">
        <div class="qr-frame">
          <QrcodeVue :value="snapshot!.challenge!.login_url" :size="188" level="H" render-as="svg" />
          <span class="qr-scan"><ScanLine :size="19" /></span>
        </div>
        <strong>{{ snapshot?.platform === 'oidc' ? '等待身份提供方确认' : '等待飞书确认' }}</strong>
        <span class="polling"><LoaderCircle class="spin" :size="14" />认证状态同步中</span>
        <button class="auth-link" type="button" :disabled="busy" @click="emit('beginQr')">
          <RotateCcw :size="14" />刷新二维码
        </button>
      </div>

      <p v-if="error" class="auth-error">{{ error }}</p>
    </section>
  </div>
</template>