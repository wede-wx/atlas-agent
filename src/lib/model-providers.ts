export type ModelProtocol = 'openai-compatible' | 'anthropic';

export type ModelProviderGroup = 'official-cn' | 'official-global' | 'aggregator' | 'local' | 'custom';

export type ModelRoutePreset = {
  id: string;
  name: string;
  protocol: ModelProtocol;
  baseUrl: string;
  authHeader?: 'authorization' | 'api-key' | 'x-api-key';
  defaultModels: string[];
  note?: string;
  requiresUserBaseUrl?: boolean;
};

export type ModelProviderPreset = {
  id: string;
  name: string;
  group: ModelProviderGroup;
  routes: ModelRoutePreset[];
};

export type ModelProviderSelection = {
  provider: ModelProviderPreset;
  route: ModelRoutePreset;
};

export const modelProviderGroups: Array<{ id: ModelProviderGroup; title: string }> = [
  { id: 'official-cn', title: '国内官方服务商' },
  { id: 'official-global', title: '国际官方服务商' },
  { id: 'aggregator', title: '聚合平台' },
  { id: 'local', title: '本地模型' },
  { id: 'custom', title: '自定义' },
];

export const modelProviderPresets: ModelProviderPreset[] = [
  {
    id: 'xiaomi-mimo',
    name: '小米 MiMo',
    group: 'official-cn',
    routes: [
      {
        id: 'mimo-standard',
        name: '普通 API',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.xiaomimimo.com/v1',
        defaultModels: ['mimo-v4-flash', 'mimo-v2.5-pro', 'mimo-v2.5'],
      },
      {
        id: 'mimo-token-plan',
        name: 'Token Plan',
        protocol: 'openai-compatible',
        baseUrl: '',
        defaultModels: ['mimo-v2.5-pro', 'mimo-v2.5', 'mimo-v2-flash'],
        requiresUserBaseUrl: true,
        note: 'Token Plan 使用专属 Base URL，请从小米 MiMo 订阅管理页复制。',
      },
    ],
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    group: 'official-cn',
    routes: [
      {
        id: 'deepseek-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.deepseek.com/v1',
        defaultModels: ['deepseek-chat', 'deepseek-reasoner'],
      },
    ],
  },
  {
    id: 'aliyun-bailian',
    name: '阿里云百炼',
    group: 'official-cn',
    routes: [
      {
        id: 'bailian-cn',
        name: '国内地域',
        protocol: 'openai-compatible',
        baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
        defaultModels: ['qwen-plus', 'qwen-max', 'qwen-turbo'],
      },
      {
        id: 'bailian-intl',
        name: '新加坡/国际',
        protocol: 'openai-compatible',
        baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1',
        defaultModels: ['qwen-plus', 'qwen-max', 'qwen-turbo'],
      },
    ],
  },
  {
    id: 'volcengine-ark',
    name: '火山方舟',
    group: 'official-cn',
    routes: [
      {
        id: 'ark-standard',
        name: '普通 API',
        protocol: 'openai-compatible',
        baseUrl: 'https://ark.cn-beijing.volces.com/api/v3',
        defaultModels: ['doubao-seed-1-6', 'doubao-1-5-pro-256k'],
      },
      {
        id: 'ark-coding-plan',
        name: 'Coding Plan',
        protocol: 'openai-compatible',
        baseUrl: 'https://ark.cn-beijing.volces.com/api/coding/v3',
        defaultModels: ['doubao-seed-1-6', 'deepseek-v3', 'deepseek-r1'],
        note: 'Coding Plan 使用专门网关，避免和普通 API 计费路径混用。',
      },
    ],
  },
  {
    id: 'zai',
    name: '智谱 AI / Z.ai',
    group: 'official-cn',
    routes: [
      {
        id: 'zai-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
        defaultModels: ['glm-5.1', 'glm-4.5', 'glm-4.5-flash'],
      },
    ],
  },
  {
    id: 'moonshot-kimi',
    name: 'Kimi / 月之暗面',
    group: 'official-cn',
    routes: [
      {
        id: 'kimi-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.moonshot.ai/v1',
        defaultModels: ['kimi-k2-0905-preview', 'moonshot-v1-128k', 'moonshot-v1-32k'],
      },
    ],
  },
  {
    id: 'baidu-qianfan',
    name: '百度千帆',
    group: 'official-cn',
    routes: [
      {
        id: 'qianfan-cn',
        name: '国内 V2',
        protocol: 'openai-compatible',
        baseUrl: 'https://qianfan.baidubce.com/v2',
        defaultModels: ['ernie-4.5-turbo-128k', 'ernie-4.0-turbo-8k', 'ernie-3.5-8k'],
      },
    ],
  },
  {
    id: 'tencent-hunyuan',
    name: '腾讯混元',
    group: 'official-cn',
    routes: [
      {
        id: 'hunyuan-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.hunyuan.cloud.tencent.com/v1',
        defaultModels: ['hunyuan-turbos-latest', 'hunyuan-standard'],
      },
    ],
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    group: 'official-cn',
    routes: [
      {
        id: 'minimax-cn',
        name: '国内版',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.minimaxi.com/v1',
        defaultModels: ['MiniMax-M2.7', 'MiniMax-M2.7-highspeed'],
      },
      {
        id: 'minimax-global',
        name: '海外版',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.minimax.io/v1',
        defaultModels: ['MiniMax-M2.7', 'MiniMax-M2.7-highspeed'],
      },
    ],
  },
  {
    id: 'siliconflow',
    name: '硅基流动',
    group: 'official-cn',
    routes: [
      {
        id: 'siliconflow-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.siliconflow.cn/v1',
        defaultModels: ['Qwen/Qwen3-Coder-480B-A35B-Instruct', 'deepseek-ai/DeepSeek-V3.2', 'moonshotai/Kimi-K2-Instruct'],
      },
    ],
  },
  {
    id: 'modelscope',
    name: 'ModelScope 魔搭',
    group: 'official-cn',
    routes: [
      {
        id: 'modelscope-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api-inference.modelscope.cn/v1',
        defaultModels: ['Qwen/Qwen3-Coder-480B-A35B-Instruct', 'deepseek-ai/DeepSeek-V3.2'],
      },
    ],
  },
  {
    id: 'iflytek-spark',
    name: '讯飞星火',
    group: 'official-cn',
    routes: [
      {
        id: 'spark-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: '',
        defaultModels: ['spark-x1', 'generalv4.5', '4.0Ultra'],
        requiresUserBaseUrl: true,
        note: '不同星火套餐的 OpenAI 兼容地址可能不同，请从控制台复制 Base URL。',
      },
    ],
  },
  {
    id: '360-ai',
    name: '360 智脑',
    group: 'official-cn',
    routes: [
      {
        id: '360-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: '',
        defaultModels: ['360gpt-pro', '360gpt-turbo'],
        requiresUserBaseUrl: true,
        note: '请从 360 智脑控制台复制当前可用的 OpenAI 兼容 Base URL。',
      },
    ],
  },
  {
    id: 'openai',
    name: 'OpenAI',
    group: 'official-global',
    routes: [
      {
        id: 'openai-default',
        name: '官方 API',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.openai.com/v1',
        defaultModels: ['gpt-5.2', 'gpt-4.1', 'gpt-4o-mini'],
      },
    ],
  },
  {
    id: 'azure-openai',
    name: 'Azure OpenAI',
    group: 'official-global',
    routes: [
      {
        id: 'azure-openai-compatible',
        name: '资源端点',
        protocol: 'openai-compatible',
        baseUrl: '',
        authHeader: 'api-key',
        defaultModels: ['gpt-5.2', 'gpt-4.1', 'gpt-4o'],
        requiresUserBaseUrl: true,
        note: 'Azure 使用你的资源专属地址，请填写包含部署路径前缀的 Base URL。',
      },
    ],
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    group: 'official-global',
    routes: [
      {
        id: 'anthropic-default',
        name: 'Claude 官方',
        protocol: 'anthropic',
        baseUrl: 'https://api.anthropic.com/v1',
        defaultModels: ['claude-sonnet-4-5', 'claude-3-5-sonnet-20241022', 'claude-3-5-haiku-20241022'],
      },
    ],
  },
  {
    id: 'github-models',
    name: 'GitHub Models',
    group: 'official-global',
    routes: [
      {
        id: 'github-models-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://models.github.ai/inference',
        defaultModels: ['openai/gpt-4.1', 'xai/grok-3-mini', 'mistral-ai/mistral-small-2503'],
      },
    ],
  },
  {
    id: 'mistral',
    name: 'Mistral',
    group: 'official-global',
    routes: [
      {
        id: 'mistral-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.mistral.ai/v1',
        defaultModels: ['mistral-large-latest', 'mistral-small-latest', 'codestral-latest'],
      },
    ],
  },
  {
    id: 'perplexity',
    name: 'Perplexity',
    group: 'official-global',
    routes: [
      {
        id: 'perplexity-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://api.perplexity.ai',
        defaultModels: ['sonar-pro', 'sonar', 'sonar-reasoning-pro'],
      },
    ],
  },
  {
    id: 'gemini',
    name: 'Gemini',
    group: 'official-global',
    routes: [
      {
        id: 'gemini-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai',
        defaultModels: ['gemini-3-pro', 'gemini-2.5-pro', 'gemini-2.5-flash'],
      },
    ],
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    group: 'aggregator',
    routes: [
      {
        id: 'openrouter-default',
        name: '统一网关',
        protocol: 'openai-compatible',
        baseUrl: 'https://openrouter.ai/api/v1',
        defaultModels: ['openai/gpt-5.2', 'anthropic/claude-sonnet-4.5', 'deepseek/deepseek-chat-v3.2'],
      },
    ],
  },
  {
    id: 'ollama',
    name: 'Ollama',
    group: 'local',
    routes: [
      {
        id: 'ollama-local',
        name: '本地服务',
        protocol: 'openai-compatible',
        baseUrl: 'http://localhost:11434/v1',
        defaultModels: ['qwen2.5:7b', 'llama3.1:8b'],
      },
    ],
  },
  {
    id: 'lmstudio',
    name: 'LM Studio',
    group: 'local',
    routes: [
      {
        id: 'lmstudio-local',
        name: '本地服务',
        protocol: 'openai-compatible',
        baseUrl: 'http://localhost:1234/v1',
        defaultModels: ['local-model'],
      },
    ],
  },
  {
    id: 'custom',
    name: '自定义',
    group: 'custom',
    routes: [
      {
        id: 'custom-openai',
        name: 'OpenAI 兼容',
        protocol: 'openai-compatible',
        baseUrl: '',
        defaultModels: ['custom-model'],
        requiresUserBaseUrl: true,
      },
      {
        id: 'custom-anthropic',
        name: 'Claude 兼容',
        protocol: 'anthropic',
        baseUrl: '',
        defaultModels: ['claude-compatible-model'],
        requiresUserBaseUrl: true,
      },
      {
        id: 'custom-gemini',
        name: 'Gemini 兼容',
        protocol: 'openai-compatible',
        baseUrl: '',
        defaultModels: ['gemini-compatible-model'],
        requiresUserBaseUrl: true,
      },
    ],
  },
];

// M1.2 静态视觉黑名单。当 provider 明确不接受 OpenAI 多模态 `image_url` content part
// 时，在前端禁用附加图片的发送路径。M5 capability matrix 完成后该列表会被结构化判定
// 替换并删除（与后端 vision_support_for_provider_id 一致）。
const VISION_UNSUPPORTED_PROVIDERS: ReadonlySet<string> = new Set(['xiaomi-mimo']);

export function isProviderVisionUnsupported(providerId?: string | null): boolean {
  return VISION_UNSUPPORTED_PROVIDERS.has(String(providerId || ''));
}

export function findModelProvider(providerId?: string | null) {
  return modelProviderPresets.find(provider => provider.id === providerId) || modelProviderPresets[0];
}

export function findModelRoute(providerId?: string | null, routeId?: string | null) {
  const provider = findModelProvider(providerId);
  const route = provider.routes.find(item => item.id === routeId) || provider.routes[0];
  return { provider, route };
}

export function firstDefaultModel(route: ModelRoutePreset) {
  return route.defaultModels[0] || 'custom-model';
}

export function sanitizeModelBaseUrl(value: string, protocol: ModelProtocol = 'openai-compatible') {
  let next = String(value || '').trim().replace(/\/+$/, '');
  if (!next) return '';
  const suffixes = protocol === 'anthropic'
    ? ['/messages', '/v1/messages', '/models', '/v1/models']
    : ['/chat/completions', '/v1/chat/completions', '/models', '/v1/models'];
  let changed = true;
  while (changed) {
    changed = false;
    for (const suffix of suffixes) {
      if (next.toLowerCase().endsWith(suffix.toLowerCase())) {
        next = next.slice(0, -suffix.length).replace(/\/+$/, '');
        changed = true;
      }
    }
  }
  return next;
}

export function endpointPreview(baseUrl: string, protocol: ModelProtocol) {
  const root = sanitizeModelBaseUrl(baseUrl, protocol);
  if (!root) return '';
  return protocol === 'anthropic' ? `${root}/messages` : `${root}/chat/completions`;
}

export function baseUrlNeedsNormalization(value: string, protocol: ModelProtocol) {
  return Boolean(value && sanitizeModelBaseUrl(value, protocol) !== value.trim().replace(/\/+$/, ''));
}

export function inferModelSelection(input: {
  provider?: string | null;
  providerId?: string | null;
  routeId?: string | null;
  baseUrl?: string | null;
  model?: string | null;
}): ModelProviderSelection {
  if (input.providerId || input.routeId) return findModelRoute(input.providerId, input.routeId);
  const base = String(input.baseUrl || '').toLowerCase();
  const model = String(input.model || '').toLowerCase();
  const legacyProvider = String(input.provider || '').toLowerCase();
  if (base.includes('token-plan') && base.includes('xiaomimimo')) return findModelRoute('xiaomi-mimo', 'mimo-token-plan');
  if (base.includes('xiaomimimo.com') || model.startsWith('mimo-')) return findModelRoute('xiaomi-mimo', 'mimo-standard');
  if (base.includes('deepseek.com') || model.includes('deepseek')) return findModelRoute('deepseek', 'deepseek-openai');
  if (base.includes('dashscope-intl')) return findModelRoute('aliyun-bailian', 'bailian-intl');
  if (base.includes('dashscope.aliyuncs.com') || model.startsWith('qwen')) return findModelRoute('aliyun-bailian', 'bailian-cn');
  if (base.includes('/api/coding')) return findModelRoute('volcengine-ark', 'ark-coding-plan');
  if (base.includes('ark.cn-beijing.volces.com') || model.includes('doubao')) return findModelRoute('volcengine-ark', 'ark-standard');
  if (base.includes('bigmodel.cn') || model.startsWith('glm-')) return findModelRoute('zai', 'zai-openai');
  if (base.includes('moonshot.ai') || model.includes('kimi') || model.includes('moonshot')) return findModelRoute('moonshot-kimi', 'kimi-openai');
  if (base.includes('qianfan') || model.includes('ernie')) return findModelRoute('baidu-qianfan', 'qianfan-cn');
  if (base.includes('hunyuan') || model.includes('hunyuan')) return findModelRoute('tencent-hunyuan', 'hunyuan-openai');
  if (base.includes('minimaxi.com')) return findModelRoute('minimax', 'minimax-cn');
  if (base.includes('minimax.io') || model.toLowerCase().startsWith('minimax-')) return findModelRoute('minimax', 'minimax-global');
  if (base.includes('siliconflow.cn')) return findModelRoute('siliconflow', 'siliconflow-openai');
  if (base.includes('modelscope.cn')) return findModelRoute('modelscope', 'modelscope-openai');
  if (model.includes('spark') || model.includes('generalv')) return findModelRoute('iflytek-spark', 'spark-openai');
  if (model.includes('360gpt')) return findModelRoute('360-ai', '360-openai');
  if (base.includes('openai.azure.com')) return findModelRoute('azure-openai', 'azure-openai-compatible');
  if (base.includes('models.github.ai')) return findModelRoute('github-models', 'github-models-openai');
  if (base.includes('api.mistral.ai') || model.includes('mistral') || model.includes('codestral')) return findModelRoute('mistral', 'mistral-openai');
  if (base.includes('perplexity.ai') || model.includes('sonar')) return findModelRoute('perplexity', 'perplexity-openai');
  if (base.includes('generativelanguage.googleapis.com') || model.includes('gemini')) return findModelRoute('gemini', 'gemini-openai');
  if (base.includes('openrouter.ai')) return findModelRoute('openrouter', 'openrouter-default');
  if (base.includes('localhost:11434') || legacyProvider === 'ollama') return findModelRoute('ollama', 'ollama-local');
  if (base.includes('localhost:1234') || legacyProvider === 'lmstudio') return findModelRoute('lmstudio', 'lmstudio-local');
  if (legacyProvider === 'anthropic' || base.includes('anthropic.com') || model.includes('claude')) return findModelRoute('anthropic', 'anthropic-default');
  if (base.includes('api.openai.com') || legacyProvider === 'openai') return findModelRoute('openai', 'openai-default');
  return findModelRoute('custom', 'custom-openai');
}
