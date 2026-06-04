export type AuraCapabilityKind = 'command' | 'skill' | 'subagent' | 'mcp';
export type AuraCapabilityRisk = 'safe' | 'sensitive' | 'destructive';
export type AuraIntentAction =
  | 'answer_only'
  | 'suggest_command'
  | 'suggest_skill'
  | 'suggest_subagent'
  | 'suggest_mcp'
  | 'ask_confirmation';

export interface AuraCapability {
  id: string;
  kind: AuraCapabilityKind;
  title: string;
  command?: string;
  slashCommands?: AuraCapabilitySlashCommand[];
  triggerWords: string[];
  description: string;
  readOnly: boolean;
  risk: AuraCapabilityRisk;
  requiresConfirmation: boolean;
  canRunParallel: boolean;
}

export interface AuraCapabilitySlashCommand {
  type?: 'command' | 'skill' | 'agent' | 'mcp';
  section: '命令' | 'Skills' | '子代理' | 'MCP';
  command: string;
  displayCommand?: string;
  aliases?: string[];
  argHint?: string;
  title: string;
  description: string;
  boundary: string;
  acceptsText?: boolean;
  skillName?: string;
  agentName?: string;
  searchText?: string;
}

export interface AuraIntentRoute {
  action: AuraIntentAction;
  capability?: AuraCapability;
  confidence: number;
  reason: string;
}

export const auraCapabilities: AuraCapability[] = [
  {
    id: 'command.code-review',
    kind: 'command',
    title: '代码审查',
    command: '/代码审查',
    slashCommands: [
      {
        section: '命令',
        type: 'agent',
        command: '/代码审查',
        aliases: ['/code-review', '/review'],
        argHint: '范围',
        title: '代码审查',
        description: '检查当前代码改动，指出 bug、风险和缺少的测试。',
        boundary: '只读审查，不修改文件。',
        acceptsText: true,
        agentName: 'code-reviewer',
        searchText: 'review code code-review 审查 代码',
      },
    ],
    triggerWords: ['代码审查', 'code review', 'review diff', '审查代码', '帮我看看代码'],
    description: '检查当前代码改动，指出 bug、风险和缺少的测试。',
    readOnly: true,
    risk: 'safe',
    requiresConfirmation: false,
    canRunParallel: true,
  },
  {
    id: 'subagent.review',
    kind: 'subagent',
    title: '只读复查子代理',
    command: '/代码审查',
    triggerWords: ['复查', '多 agent', '子代理', '检查一下', 'reviewer'],
    description: '需要独立审查时建议使用只读代码审查子代理。',
    readOnly: true,
    risk: 'safe',
    requiresConfirmation: false,
    canRunParallel: true,
  },
  {
    id: 'mcp.manage',
    kind: 'mcp',
    title: 'MCP 服务',
    command: '/mcp',
    slashCommands: [
      {
        section: 'MCP',
        type: 'mcp',
        command: '/mcp',
        title: 'MCP 服务',
        description: '查看 MCP 服务状态和管理入口。',
        boundary: '只显示说明，不调用外部工具。',
      },
    ],
    triggerWords: ['mcp', 'server', '工具服务', '外部工具', 'resources', 'prompts'],
    description: '配置、测试和审计 MCP server；危险工具调用需要确认。',
    readOnly: false,
    risk: 'sensitive',
    requiresConfirmation: true,
    canRunParallel: false,
  },
  {
    id: 'command.dangerous-local-action',
    kind: 'command',
    title: '高风险本地操作',
    triggerWords: ['删除文件', '永久删除', '重置', '清空', '覆盖', '执行命令', 'run command'],
    description: '本地写入、删除、命令和 MCP 写入类动作必须先确认。',
    readOnly: false,
    risk: 'destructive',
    requiresConfirmation: true,
    canRunParallel: false,
  },
];

export const auraCapabilitySlashCommands: AuraCapabilitySlashCommand[] =
  auraCapabilities.flatMap(capability => capability.slashCommands ?? []);

export function routeAuraIntent(message: string): AuraIntentRoute {
  const text = message.trim().toLowerCase();
  if (!text) {
    return { action: 'answer_only', confidence: 0, reason: '空输入。' };
  }
  if (text.startsWith('/')) {
    return { action: 'answer_only', confidence: 1, reason: '用户已经显式输入命令。' };
  }

  const matches = auraCapabilities
    .map(capability => ({
      capability,
      score: capability.triggerWords.reduce((score, trigger) => (
        text.includes(trigger.toLowerCase()) ? score + Math.max(1, trigger.length / 3) : score
      ), 0),
    }))
    .filter(match => match.score > 0)
    .sort((a, b) => b.score - a.score);

  const best = matches[0];
  if (!best) {
    return { action: 'answer_only', confidence: 0.2, reason: '没有命中任何可自动建议的能力。' };
  }

  if (best.capability.requiresConfirmation || best.capability.risk !== 'safe') {
    return {
      action: best.capability.kind === 'mcp' ? 'suggest_mcp' : 'ask_confirmation',
      capability: best.capability,
      confidence: Math.min(0.96, 0.55 + best.score / 10),
      reason: `${best.capability.title} 涉及${best.capability.risk === 'destructive' ? '高风险' : '敏感'}能力，只能建议或等待确认。`,
    };
  }

  const action: AuraIntentAction =
    best.capability.kind === 'skill'
      ? 'suggest_skill'
      : best.capability.kind === 'subagent'
        ? 'suggest_subagent'
        : best.capability.kind === 'mcp'
          ? 'suggest_mcp'
          : 'suggest_command';

  return {
    action,
    capability: best.capability,
    confidence: Math.min(0.96, 0.5 + best.score / 10),
    reason: `当前输入匹配 ${best.capability.title}，先给出建议，不自动执行。`,
  };
}
