<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { buildClientConfig, copyClientConfig } from './config'

type ToolSummary = { name: string; description: string }
type ServiceSnapshot = {
  phase: 'starting' | 'running' | 'stopped' | 'failed'
  url: string | null
  token: string | null
  tools: ToolSummary[]
  message: string | null
}

const snapshot = ref<ServiceSnapshot>({ phase: 'starting', url: null, token: null, tools: [], message: null })
const busy = ref(false)
const feedback = ref('正在连接本地服务…')
const visibleTools = ref(false)
let unlisten: UnlistenFn | undefined

const isRunning = computed(() => snapshot.value.phase === 'running' && Boolean(snapshot.value.url && snapshot.value.token))
const configText = computed(() => isRunning.value
  ? buildClientConfig(snapshot.value.url!, snapshot.value.token!)
  : '{\n  "mcpServers": {}\n}')
const statusLabel = computed(() => ({
  starting: '正在启动',
  running: '服务运行中',
  stopped: '服务已停止',
  failed: '服务启动失败',
}[snapshot.value.phase]))
const toolCount = computed(() => snapshot.value.tools.length)

async function start(): Promise<void> {
  busy.value = true
  snapshot.value = { ...snapshot.value, phase: 'starting', message: null }
  feedback.value = '正在启动本地 MCP 服务…'
  try {
    snapshot.value = await invoke<ServiceSnapshot>('start_service')
    feedback.value = '服务已启动。复制下方 JSON 到 MCP 客户端即可连接。'
  } catch (error) {
    snapshot.value = { phase: 'failed', url: null, token: null, tools: [], message: String(error) }
    feedback.value = `启动失败：${String(error)}`
  } finally {
    busy.value = false
  }
}

async function stop(): Promise<void> {
  busy.value = true
  try {
    snapshot.value = await invoke<ServiceSnapshot>('stop_service')
    feedback.value = '本地 MCP 服务已停止。'
  } catch (error) {
    feedback.value = `停止失败：${String(error)}`
  } finally {
    busy.value = false
  }
}

async function restart(rotate = false): Promise<void> {
  busy.value = true
  feedback.value = rotate ? '正在更换 Token…' : '正在重启本地服务…'
  try {
    if (snapshot.value.phase !== 'stopped') {
      snapshot.value = await invoke<ServiceSnapshot>('stop_service')
    }
    snapshot.value = await invoke<ServiceSnapshot>('start_service')
    feedback.value = rotate ? 'Token 已更新，旧配置已失效。请复制新配置。' : '服务已重新启动。'
  } catch (error) {
    snapshot.value = { phase: 'failed', url: null, token: null, tools: [], message: String(error) }
    feedback.value = `操作失败：${String(error)}`
  } finally {
    busy.value = false
  }
}

async function copyConfig(): Promise<void> {
  const result = await copyClientConfig(configText.value, (content) => invoke('copy_config', { content }))
  feedback.value = result.message
}

onMounted(async () => {
  unlisten = await listen<ServiceSnapshot>('service-state', (event) => {
    snapshot.value = event.payload
    if (event.payload.phase === 'failed') feedback.value = event.payload.message ?? '服务异常退出。'
  })
  try {
    const current = await invoke<ServiceSnapshot>('current_service')
    snapshot.value = current
    if (current.phase !== 'running') await start()
  } catch (error) {
    snapshot.value = { phase: 'failed', url: null, token: null, tools: [], message: String(error) }
    feedback.value = `启动失败：${String(error)}`
  }
})

onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <main class="shell">
    <header class="page-header">
      <div class="brand-mark" aria-hidden="true"><span></span><span></span><span></span></div>
      <div>
        <p class="eyebrow">AGENT-DESKTOP · LOCAL MCP</p>
        <h1>MCP 地址与客户端配置</h1>
      </div>
      <span class="status-pill" :class="`status-${snapshot.phase}`">
        <span class="status-dot" aria-hidden="true"></span>{{ statusLabel }}
      </span>
    </header>

    <section class="card connection-card" aria-labelledby="connection-title">
      <div class="section-heading">
        <div>
          <p class="eyebrow accent">连接信息</p>
          <h2 id="connection-title">本机服务</h2>
        </div>
        <span class="local-badge">仅本机可访问</span>
      </div>

      <div class="field-group">
        <label for="service-url">MCP 服务地址</label>
        <div class="field-value url-value" id="service-url">{{ snapshot.url ?? 'http://127.0.0.1:12899/desktop' }}</div>
        <p class="helper">此地址供运行在本机的 MCP 客户端使用。</p>
      </div>

      <div class="field-group token-group">
        <label for="service-token">当前访问 Token</label>
        <div class="field-value token-value" id="service-token">{{ snapshot.token ?? '等待服务启动…' }}</div>
        <p class="helper">更换 Token 后，旧配置会立即失效，需要更新 MCP 客户端。</p>
      </div>

      <div v-if="snapshot.message" class="error-note" role="alert">{{ snapshot.message }}</div>

      <div class="actions">
        <button class="button button-primary" type="button" :disabled="busy || !isRunning" @click="copyConfig">
          <span aria-hidden="true">⧉</span> 复制客户端配置
        </button>
        <button class="button button-quiet" type="button" :disabled="busy" @click="restart(false)">
          重启服务
        </button>
        <button class="button button-quiet" type="button" :disabled="busy || !isRunning" @click="restart(true)">
          更换 Token
        </button>
        <button v-if="isRunning" class="button button-danger" type="button" :disabled="busy" @click="stop">
          停止服务
        </button>
        <button v-else class="button button-quiet" type="button" :disabled="busy" @click="start">
          启动服务
        </button>
      </div>
      <p class="live-feedback" role="status" aria-live="polite" aria-atomic="true">{{ feedback }}</p>
    </section>

    <section class="card config-card" aria-labelledby="config-title">
      <div class="section-heading">
        <div>
          <p class="eyebrow accent">一键复制使用</p>
          <h2 id="config-title">客户端 JSON 配置</h2>
        </div>
        <span class="format-tag">JSON</span>
      </div>
      <pre class="code-panel"><code>{{ configText }}</code></pre>
      <p class="helper config-helper">将这段内容粘贴到支持 Streamable HTTP 的 MCP 客户端配置中。</p>
    </section>

    <section class="card tools-card" aria-labelledby="tools-title">
      <div class="tools-heading">
        <div>
          <p class="eyebrow accent">已从当前 CLI 加载</p>
          <h2 id="tools-title">可用工具 <strong>{{ toolCount }}</strong></h2>
        </div>
        <button class="disclosure" type="button" :aria-expanded="visibleTools" @click="visibleTools = !visibleTools">
          {{ visibleTools ? '收起列表' : '查看工具列表' }}
          <span aria-hidden="true">{{ visibleTools ? '−' : '+' }}</span>
        </button>
      </div>
      <p v-if="!toolCount" class="helper">服务启动后会显示可用工具数量和说明。</p>
      <ul v-if="visibleTools && toolCount" class="tool-list">
        <li v-for="tool in snapshot.tools" :key="tool.name">
          <code>{{ tool.name }}</code><span>{{ tool.description }}</span>
        </li>
      </ul>
    </section>

    <footer class="page-footer">访问凭据只在服务运行期间保存在内存中。</footer>
  </main>
</template>
