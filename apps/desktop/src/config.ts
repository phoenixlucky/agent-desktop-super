export type CopyFeedback = {
  state: 'success' | 'error'
  message: string
}

export function buildClientConfig(url: string, token: string, serverName = 'agent-desktop'): string {
  return JSON.stringify({
    mcpServers: {
      [serverName]: {
        type: 'streamable-http',
        url,
        headers: { Authorization: `Bearer ${token}` },
      },
    },
  }, null, 2)
}

export async function copyClientConfig(
  text: string,
  writeText: (value: string) => Promise<void>,
): Promise<CopyFeedback> {
  try {
    await writeText(text)
    return { state: 'success', message: '配置已复制，可以粘贴到 MCP 客户端。' }
  } catch {
    return { state: 'error', message: '复制失败，请检查系统剪贴板权限后重试。' }
  }
}
