import { describe, expect, it, vi } from 'vitest'
import { buildClientConfig, copyClientConfig } from './config'

describe('MCP client configuration', () => {
  it('prints the local Streamable HTTP configuration in the client format', () => {
    const config = buildClientConfig('http://127.0.0.1:12899/desktop', 'token-value')
    expect(config).toBe(JSON.stringify({
      mcpServers: {
        'agent-desktop': {
          type: 'streamable-http',
          url: 'http://127.0.0.1:12899/desktop',
          headers: { Authorization: 'Bearer token-value' },
        },
      },
    }, null, 2))
  })

  it('escapes values before placing them in JSON', () => {
    const config = buildClientConfig('http://127.0.0.1:12899/desktop', 'token"\\\n', 'agent"desktop')
    expect(() => JSON.parse(config)).not.toThrow()
    expect(JSON.parse(config).mcpServers['agent"desktop'].headers.Authorization).toBe('Bearer token"\\\n')
  })

  it('returns visible success and failure feedback from clipboard writes', async () => {
    const success = await copyClientConfig('{}', vi.fn().mockResolvedValue(undefined))
    const failure = await copyClientConfig('{}', vi.fn().mockRejectedValue(new Error('clipboard blocked')))
    expect(success).toEqual({ state: 'success', message: '配置已复制，可以粘贴到 MCP 客户端。' })
    expect(failure).toEqual({ state: 'error', message: '复制失败，请检查系统剪贴板权限后重试。' })
  })
})
