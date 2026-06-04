export type ModelContextSource = 'detected' | 'user_config' | 'builtin' | 'fallback';

export interface ModelContextWindow {
  total: number;
  source: ModelContextSource;
  provider: string;
  model: string;
  label: string;
}

const CONTEXT_STORAGE_KEY = 'aura_model_context_windows_v1';

const SOURCE_LABEL: Record<ModelContextSource, string> = {
  detected: '检测结果',
  user_config: '用户配置',
  builtin: '模型默认',
  fallback: '保守默认',
};

const MODEL_PATTERNS: Array<{ match: RegExp; total: number }> = [
  { match: /gpt-4\.1|gpt-4-1/i, total: 1_047_576 },
  { match: /gpt-4o|gpt-4\.o|o1|o3|o4/i, total: 128_000 },
  { match: /gpt-5/i, total: 400_000 },
  { match: /claude/i, total: 200_000 },
  { match: /gemini/i, total: 1_000_000 },
  { match: /deepseek|deepseek-v/i, total: 128_000 },
  { match: /qwen|qwq|通义/i, total: 131_072 },
  { match: /kimi|moonshot/i, total: 128_000 },
  { match: /llama|mistral|mixtral|yi-|glm|doubao/i, total: 128_000 },
];

const PROVIDER_FALLBACKS: Record<string, number> = {
  anthropic: 200_000,
  openai: 128_000,
  ollama: 32_768,
  lmstudio: 32_768,
};

function normalizeKey(provider: string, model: string) {
  return `${provider || 'unknown'}::${model || 'unknown'}`.toLowerCase();
}

function readStoredWindows(): Record<string, ModelContextWindow> {
  try {
    const raw = window.localStorage.getItem(CONTEXT_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : {};
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

export function storeModelContextWindow(windowInfo: ModelContextWindow) {
  if (!windowInfo.total || windowInfo.total <= 0) return;
  try {
    const windows = readStoredWindows();
    windows[normalizeKey(windowInfo.provider, windowInfo.model)] = windowInfo;
    window.localStorage.setItem(CONTEXT_STORAGE_KEY, JSON.stringify(windows));
  } catch {
    // localStorage can be unavailable in tests or privacy-restricted WebViews.
  }
}

export function extractContextWindow(raw: unknown): number | null {
  const candidates = new Set(['context_length', 'contextWindow', 'context_window', 'max_context_length', 'max_model_len', 'input_token_limit', 'max_input_tokens']);
  const seen = new Set<unknown>();

  const walk = (value: unknown): number | null => {
    if (!value || typeof value !== 'object' || seen.has(value)) return null;
    seen.add(value);
    if (Array.isArray(value)) {
      for (const item of value) {
        const found = walk(item);
        if (found) return found;
      }
      return null;
    }
    for (const [key, fieldValue] of Object.entries(value as Record<string, unknown>)) {
      if (candidates.has(key)) {
        const numeric = typeof fieldValue === 'number' ? fieldValue : Number(fieldValue);
        if (Number.isFinite(numeric) && numeric > 0) return Math.floor(numeric);
      }
      const nested = walk(fieldValue);
      if (nested) return nested;
    }
    return null;
  };

  return walk(raw);
}

export function resolveModelContextWindow(
  provider: string,
  modelName: string,
  options: { detectedTotal?: number | null; configuredTotal?: number | null } = {},
): ModelContextWindow {
  const model = (modelName || '').trim();
  const detectedTotal = Number(options.detectedTotal || 0);
  if (detectedTotal > 0) {
    return {
      total: Math.floor(detectedTotal),
      source: 'detected',
      provider,
      model,
      label: SOURCE_LABEL.detected,
    };
  }

  const configuredTotal = Number(options.configuredTotal || 0);
  if (configuredTotal > 0) {
    return {
      total: Math.floor(configuredTotal),
      source: 'user_config',
      provider,
      model,
      label: SOURCE_LABEL.user_config,
    };
  }

  const stored = readStoredWindows()[normalizeKey(provider, model)];
  if (stored?.total > 0) return stored;

  const pattern = MODEL_PATTERNS.find(item => item.match.test(model));
  if (pattern) {
    return {
      total: pattern.total,
      source: 'builtin',
      provider,
      model,
      label: SOURCE_LABEL.builtin,
    };
  }

  const fallback = PROVIDER_FALLBACKS[(provider || '').toLowerCase()] || 128_000;
  return {
    total: fallback,
    source: 'fallback',
    provider,
    model,
    label: SOURCE_LABEL.fallback,
  };
}
