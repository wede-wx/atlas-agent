import { create } from 'zustand';

import { EventBus } from '../lib/event-bus';
import {
  addMemory as addMemoryDb,
  archiveSession as archiveSessionDb,
  clearMemories as clearMemoriesDb,
  createSession as createSessionDb,
  deleteMemory as deleteMemoryDb,
  deleteSession as deleteSessionDb,
  getAppState,
  getArchivedSessions,
  getErrorMessage,
  getMemories,
  getMessages,
  getProfile,
  getSessions,
  initLocalDb,
  logAuraActivity,
  isTauriRuntime,
  renameSession as renameSessionDb,
  restoreSession as restoreSessionDb,
  saveMessage as saveMessageDb,
  saveProfile,
  setAppState,
  setSessionPinned as setSessionPinnedDb,
  updateMemory as updateMemoryDb,
  type MemoryRecord,
  type MessageRecord,
  type ProfileRecord,
  type SessionRecord,
} from '../lib/invoke-bridge';

export type Level = 'L1' | 'L2' | 'L3';
export type PermissionMode = 'plan' | 'default' | 'full_access';
export type ReplyStyle = 'gentle' | 'professional' | 'minimal';
export type ToastMsg = { id: number; msg: string; type: 'success' | 'warning' | 'error' | 'info' };

export interface AppWindow {
  instanceId: string;
  appType: string;
  title: string;
  zIndex: number;
  isFocused: boolean;
  offsetX: number;
  offsetY: number;
  width: number;
  height: number;
  prevOffsetX?: number;
  prevOffsetY?: number;
  prevWidth?: number;
  prevHeight?: number;
  isMinimized: boolean;
  isMaximized: boolean;
  props: any;
}

export interface ChatMessage {
  id: string;
  role: 'system' | 'user' | 'aura' | 'tool';
  content: string;
  timestamp: number;
  metadata?: any;
  streaming?: boolean;
  error?: boolean;
}

export interface MemoryItem {
  id: string;
  text: string;
  createdAt: number;
  enabled: boolean;
  source?: string;
  quality?: string;
  confidence?: number;
  lastUsedAt?: number | null;
  useCount?: number;
}

type Session = {
  id: string;
  title: string;
  time: string;
  messages: ChatMessage[];
  projectId?: string | null;
  titleIsManual?: boolean;
  pinned?: boolean;
  archivedAt?: number | null;
  createdAt?: number;
  lastActiveAt?: number;
};
type WindowRect = { offsetX: number; offsetY: number; width: number; height: number };
type ToastMsgWithCreatedAt = ToastMsg & { createdAt?: number };

const defaultWindowMinSize = { width: 360, height: 500 };
const allowedAppTypes = new Set(['chat', 'profile', 'settings']);
const appWindowMinSizes: Record<string, { width: number; height: number }> = {
  chat: { width: 1180, height: 680 },
  profile: { width: 1180, height: 760 },
  settings: { width: 1040, height: 720 },
};

const defaultUser = {
  id: `GUEST-${Date.now().toString().slice(-4)}`,
  email: '',
  nickname: '主人',
  replyStyle: 'professional' as ReplyStyle,
  inviteCode: 'AURA-LOCAL',
  inviteCount: 0,
  systemPrompt: '你正在和 Aura 对话。请记住用户的长期偏好，并保持风格一致。',
  signature: '在 Aura 里整理自己的数字画像',
  interests: ['科幻', '电子音乐', '深夜编程'],
  soulPrompt: '# Aura Soul\n语气温和、清晰、有边界。',
  profile: {
    interests: '科幻、电子音乐、数字艺术',
    tone: '简洁',
    title: '主人',
  },
};

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`aura_${key}`);
    return raw ? JSON.parse(raw) : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, val: any) {
  try {
    localStorage.setItem(`aura_${key}`, JSON.stringify(val));
  } catch {
    // Local UI preferences are best-effort.
  }
}

function windowLayoutKey(appType: string) {
  return `windowLayout_${appType}`;
}

function saveWindowLayout(window: Pick<AppWindow, 'appType' | 'offsetX' | 'offsetY' | 'width' | 'height'>) {
  save(windowLayoutKey(window.appType), {
    offsetX: window.offsetX,
    offsetY: window.offsetY,
    width: window.width,
    height: window.height,
  });
}

function viewportSize() {
  if (typeof window === 'undefined') return { width: 1440, height: 900 };
  return {
    width: Math.max(760, window.innerWidth || 1440),
    height: Math.max(640, window.innerHeight || 900),
  };
}

function maximizedWindowRect(): WindowRect {
  const viewport = viewportSize();
  const safeLeft = 58;
  const safeTop = 50;
  const safeRight = 16;
  const safeBottom = 18;
  return {
    offsetX: safeLeft,
    offsetY: safeTop,
    width: Math.max(360, viewport.width - safeLeft - safeRight),
    height: Math.max(500, viewport.height - safeTop - safeBottom),
  };
}

function windowMinSize(appType?: string) {
  return appType ? appWindowMinSizes[appType] || defaultWindowMinSize : defaultWindowMinSize;
}

function defaultWindowSize(appType: string) {
  return {
    width: appType === 'chat' ? 1280 : appType === 'profile' ? 1240 : appType === 'settings' ? 1080 : 920,
    height: appType === 'chat' ? 780 : ['profile', 'settings'].includes(appType) ? 760 : 660,
  };
}

function centeredWindowRect(appType: string, count = 0): WindowRect {
  const viewport = viewportSize();
  const safeLeft = 58;
  const safeTop = 50;
  const safeRight = 16;
  const safeBottom = 18;
  const desired = defaultWindowSize(appType);
  const availableWidth = Math.max(defaultWindowMinSize.width, viewport.width - safeLeft - safeRight);
  const availableHeight = Math.max(defaultWindowMinSize.height, viewport.height - safeTop - safeBottom);
  const minSize = windowMinSize(appType);
  const width = Math.min(Math.max(minSize.width, desired.width), availableWidth);
  const height = Math.min(Math.max(minSize.height, desired.height), availableHeight);
  const shift = (count % 4) * 14;
  return clampWindowRect({
    offsetX: Math.round(safeLeft + (availableWidth - width) / 2 + shift),
    offsetY: Math.round(safeTop + (availableHeight - height) / 2 + shift),
    width,
    height,
  }, appType);
}

function placedWindowRect(appType: string, count = 0, _placement?: string): WindowRect {
  return centeredWindowRect(appType, count);
}

function clampWindowRect(rect: WindowRect, appType?: string): WindowRect {
  const viewport = viewportSize();
  const safeLeft = 58;
  const safeTop = 50;
  const safeRight = 16;
  const safeBottom = 18;
  const minSize = windowMinSize(appType);
  const availableWidth = Math.max(defaultWindowMinSize.width, viewport.width - safeLeft - safeRight);
  const availableHeight = Math.max(defaultWindowMinSize.height, viewport.height - safeTop - safeBottom);
  const width = Math.min(Math.max(minSize.width, rect.width), availableWidth);
  const height = Math.min(Math.max(minSize.height, rect.height), availableHeight);
  const maxX = Math.max(safeLeft, viewport.width - safeRight - width);
  const maxY = Math.max(safeTop, viewport.height - safeBottom - height);
  return {
    offsetX: Math.min(Math.max(safeLeft, rect.offsetX), maxX),
    offsetY: Math.min(Math.max(safeTop, rect.offsetY), maxY),
    width,
    height,
  };
}

function normalizeProvider(value: unknown): string {
  return String(value || 'openai');
}

function normalizePermissionMode(value: unknown): PermissionMode {
  const raw = String(value || '').trim();
  if (raw === 'safe' || raw === 'suggest' || raw === 'plan') return 'plan';
  if (raw === 'full' || raw === 'full_auto' || raw === 'full_access') return 'full_access';
  return 'default';
}

function permissionModeLabel(value: PermissionMode) {
  if (value === 'plan') return '计划模式';
  if (value === 'full_access') return '完全访问模式';
  return '默认模式';
}

function roleFromDb(role: string): ChatMessage['role'] {
  if (role === 'assistant') return 'aura';
  if (role === 'user' || role === 'system' || role === 'tool') return role;
  return 'system';
}

function roleToDb(role: ChatMessage['role']) {
  return role === 'aura' ? 'assistant' : role;
}

function mapSession(record: SessionRecord, messages: ChatMessage[] = []): Session {
  return {
    id: record.id,
    title: record.title || '新会话',
    time: new Date(record.last_active_at || record.updated_at).toLocaleString(),
    messages,
    projectId: record.project_id || null,
    titleIsManual: Boolean(record.title_is_manual),
    pinned: Boolean(record.pinned),
    archivedAt: record.archived_at ?? null,
    createdAt: record.created_at,
    lastActiveAt: record.last_active_at || record.updated_at,
  };
}

function mapMessage(record: MessageRecord): ChatMessage {
  return {
    id: record.id,
    role: roleFromDb(record.role),
    content: record.content,
    timestamp: record.created_at,
    metadata: record.metadata,
  };
}

function mapMemory(record: MemoryRecord): MemoryItem {
  return {
    id: record.id,
    text: record.text,
    createdAt: record.createdAt ?? record.created_at ?? Date.now(),
    enabled: record.enabled,
    source: record.source,
    quality: record.quality,
    confidence: record.confidence,
    lastUsedAt: record.lastUsedAt ?? null,
    useCount: record.useCount ?? 0,
  };
}

async function mapSessionRecords(records: SessionRecord[]): Promise<Array<[string, Session]>> {
  const entries: Array<[string, Session]> = [];
  for (const record of records) {
    const messages = await getMessages(record.id).catch(() => [] as MessageRecord[]);
    entries.push([record.id, mapSession(record, messages.map(mapMessage))]);
  }
  return entries;
}

function normalizeMemories(raw: unknown): Array<{ text: string; enabled: boolean }> {
  if (!Array.isArray(raw)) return [];
  return raw
    .map((item) => {
      if (typeof item === 'string') return { text: item, enabled: true };
      const candidate = item as Partial<MemoryItem>;
      return { text: String(candidate.text || ''), enabled: candidate.enabled ?? true };
    })
    .filter((memory) => memory.text.trim());
}

function normalizeSessions(raw: unknown): Record<string, Session> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw as Record<string, any>)
      .filter(([, value]) => value && typeof value === 'object')
      .map(([id, value]) => [
        id,
        {
          id: value.id || id,
          title: value.title || '旧会话',
          time: value.time || '本地缓存',
          messages: Array.isArray(value.messages) ? value.messages : [],
          projectId: value.projectId || value.project_id || null,
          titleIsManual: Boolean(value.titleIsManual || value.title_is_manual),
          pinned: Boolean(value.pinned),
          archivedAt: value.archivedAt || value.archived_at || null,
          createdAt: Number(value.createdAt || value.created_at || Date.now()),
          lastActiveAt: Number(value.lastActiveAt || value.last_active_at || Date.now()),
        },
      ]),
  );
}

function normalizeProfileRecord(record: ProfileRecord | null | undefined) {
  const profile = record?.profile || {};
  return {
    profile,
    userPatch: {
      nickname: profile.nickname || defaultUser.nickname,
      signature: profile.signature || defaultUser.signature,
      interests: Array.isArray(profile.tags) ? profile.tags : defaultUser.interests,
      replyStyle: profile.replyStyle || defaultUser.replyStyle,
      systemPrompt: profile.systemPrompt || defaultUser.systemPrompt,
      soulPrompt: profile.soulPrompt || defaultUser.soulPrompt,
      personalityProfile: profile.personalityProfile || {},
      profile: {
        interests: Array.isArray(profile.interests) ? profile.interests.join('、') : (profile.interests || defaultUser.profile.interests),
        tone: profile.tonePreference || defaultUser.profile.tone,
        title: profile.titlePreference || defaultUser.profile.title,
      },
    },
  };
}

function titleFromMessage(content: string) {
  const compact = content.replace(/\s+/g, ' ').trim();
  if (!compact) return '新会话';
  const prefix = compact.length > 18 ? `${compact.slice(0, 18)}...` : compact;
  return prefix.replace(/[。！？!?，,]$/, '') || '新会话';
}

function isDuplicateMessage(messages: ChatMessage[], msg: ChatMessage) {
  return messages.some((item) => {
    if (item.id === msg.id) return true;
    if (item.role !== msg.role || item.content !== msg.content) return false;
    return Math.abs(item.timestamp - msg.timestamp) < 1200;
  });
}

export const useSystemStore = create<{
  theme: 'dark' | 'light';
  cacheSize: number;
  isOnline: boolean;
  provider: string;
  apiUrl: string;
  apiKey: string;
  modelName: string;
  notifType: string;
  startupType: string;
  soundEnabled: boolean;
  toggleTheme: () => void;
  setTheme: (theme: 'dark' | 'light') => void;
  clearCache: () => void;
  setOnline: (o: boolean) => void;
  updateConfig: (cfg: any) => void;
  toggleSound: () => void;
}>((set, get) => ({
  theme: load('theme', 'dark'),
  cacheSize: load('cache', 1250),
  isOnline: typeof navigator !== 'undefined' ? navigator.onLine : true,
  provider: normalizeProvider(load('provider', 'openai')),
  apiUrl: load('apiUrl', ''),
  apiKey: '',
  modelName: load('modelName', 'gpt-4o-mini'),
  notifType: load('notif', '0'),
  startupType: load('startup', '0'),
  soundEnabled: false,
  toggleTheme: () => {
    const next = get().theme === 'dark' ? 'light' : 'dark';
    save('theme', next);
    set({ theme: next });
  },
  setTheme: (theme) => {
    save('theme', theme);
    set({ theme });
  },
  setOnline: (isOnline) => set({ isOnline }),
  updateConfig: (cfg) => {
    const normalized = { ...cfg, provider: cfg.provider ? normalizeProvider(cfg.provider) : cfg.provider };
    Object.entries(normalized).forEach(([key, value]) => {
      if (value === undefined) return;
      if (key === 'apiKey') {
        return;
      }
      if (['theme', 'soundEnabled', 'provider', 'apiUrl', 'modelName', 'startupType', 'notifType', 'cacheSize'].includes(key)) {
        const storageKey = key === 'soundEnabled' ? 'sound' : key === 'startupType' ? 'startup' : key === 'notifType' ? 'notif' : key === 'cacheSize' ? 'cache' : key;
        save(storageKey, value);
      }
    });
    set(normalized as any);
  },
  toggleSound: () => {
    save('sound', false);
    set({ soundEnabled: false });
  },
  clearCache: () => {
    set({ cacheSize: 0 });
    save('cache', 0);
    EventBus.dispatch('TOAST', { msg: '已清理本地界面缓存，核心数据未删除。', type: 'success' });
  },
}));

export const useAuthStore = create<{
  isAuthenticated: boolean;
  level: Level;
  permissionMode: PermissionMode;
  user: typeof defaultUser;
  login: (email: string) => void;
  logout: () => void;
  setLevel: (l: Level) => void;
  setPermissionMode: (mode: PermissionMode) => Promise<void>;
  updatePersona: (cfg: any) => Promise<void>;
  hydrateProfile: () => Promise<void>;
}>((set, get) => ({
  isAuthenticated: true,
  level: load('level', 'L1'),
  permissionMode: normalizePermissionMode(load('permissionMode', 'default')),
  user: load('user', defaultUser),
  login: (email) => set((state) => {
    const nextUser = { ...state.user, email, id: `AURA-${Date.now().toString().slice(-6)}` };
    save('user', nextUser);
    return { isAuthenticated: true, user: nextUser };
  }),
  logout: () => {
    set({ isAuthenticated: false });
    EventBus.dispatch('TOAST', { msg: '已退出本地预览身份，数据仍保留在本机。', type: 'info' });
  },
  setLevel: (level) => {
    save('level', level);
    set({ level });
    if (isTauriRuntime()) setAppState('preview_level', level).catch(() => {});
  },
  setPermissionMode: async (permissionMode) => {
    const normalized = normalizePermissionMode(permissionMode);
    const previous = get().permissionMode;
    save('permissionMode', normalized);
    set({ permissionMode: normalized });
    if (isTauriRuntime()) {
      try {
        await setAppState('agent_permission_mode', normalized);
      } catch (error) {
        save('permissionMode', previous);
        set({ permissionMode: previous });
        EventBus.dispatch('TOAST', { msg: `权限切换失败：${getErrorMessage(error)}`, type: 'error' });
        throw error;
      }
    }
    const label = permissionModeLabel(normalized);
    EventBus.dispatch('TOAST', { msg: `Agent 权限已切换为：${label}。高风险动作仍会要求确认。`, type: normalized === 'full_access' ? 'warning' : 'info' });
  },
  updatePersona: async (cfg) => {
    const previous = get().user;
    const nextUser = { ...previous, ...cfg };
    save('user', nextUser);
    set({ user: nextUser });
    if (!isTauriRuntime()) return;
    try {
      await saveProfile({
        nickname: nextUser.nickname || '',
        signature: nextUser.signature || '',
        interests: nextUser.profile?.interests || '',
        tags: nextUser.interests || [],
        tonePreference: nextUser.profile?.tone || '',
        titlePreference: nextUser.profile?.title || '',
        replyStyle: nextUser.replyStyle,
        systemPrompt: nextUser.systemPrompt,
        soulPrompt: nextUser.soulPrompt,
        personalityProfile: nextUser.personalityProfile || {},
        profile: nextUser.profile || {},
      });
    } catch (error) {
      save('user', previous);
      set({ user: previous });
      EventBus.dispatch('TOAST', { msg: `个性化资料保存失败：${getErrorMessage(error)}`, type: 'error' });
      throw error;
    }
  },
  hydrateProfile: async () => {
    if (!isTauriRuntime()) return;
    const [level, storedPermissionMode, openedFullMigration, profile] = await Promise.all([
      getAppState<Level>('preview_level').catch(() => null),
      getAppState<PermissionMode>('agent_permission_mode').catch(() => null),
      getAppState<boolean>('agent_permission_full_opened_20260506').catch(() => null),
      getProfile().catch(() => null),
    ]);
    const permissionMode = normalizePermissionMode(openedFullMigration ? storedPermissionMode : 'default');
    if (!openedFullMigration) {
      save('permissionMode', permissionMode);
      await Promise.all([
        setAppState('agent_permission_mode', permissionMode).catch(() => {}),
        setAppState('agent_permission_full_opened_20260506', true).catch(() => {}),
      ]);
    }
    const patch = normalizeProfileRecord(profile);
    set((state) => ({
      level: level || state.level,
      permissionMode,
      user: { ...state.user, ...patch.userPatch },
    }));
  },
}));

export const useUIStore = create<{
  toasts: ToastMsg[];
  cmdOpen: boolean;
  addToast: (msg: string, type?: ToastMsg['type']) => void;
  setCmdOpen: (val: boolean | ((prev: boolean) => boolean)) => void;
  contextMenu: { x: number; y: number; show: boolean } | null;
  setContextMenu: (m: any) => void;
}>((set) => ({
  toasts: [],
  cmdOpen: false,
  contextMenu: null,
  addToast: (msg, type = 'info') => {
    const createdAt = Date.now();
    const id = createdAt + Math.random();
    let accepted = false;
    set((state) => {
      const duplicated = (state.toasts as ToastMsgWithCreatedAt[]).some((toast) => {
        const toastCreatedAt = toast.createdAt ?? Math.floor(toast.id);
        return toast.msg === msg && toast.type === type && createdAt - toastCreatedAt < 1800;
      });
      if (duplicated) return state;
      accepted = true;
      return { toasts: [...state.toasts.slice(-4), { id, msg, type, createdAt } as ToastMsgWithCreatedAt] };
    });
    if (!accepted) return;
    setTimeout(() => set((state) => ({ toasts: state.toasts.filter(toast => toast.id !== id) })), 3000);
  },
  setCmdOpen: (val) => set((state) => ({ cmdOpen: typeof val === 'function' ? val(state.cmdOpen) : val })),
  setContextMenu: (contextMenu) => set({ contextMenu }),
}));

export const useWMStore = create<{
  windows: AppWindow[];
  baseZIndex: number;
  openApp: (t: string, title: string, props?: any) => void;
  closeWindow: (id: string) => void;
  focusWindow: (id: string) => void;
  restoreWindow: (id: string) => void;
  updateWindowPosition: (id: string, x: number, y: number) => void;
  updateWindowSize: (id: string, w: number, h: number) => void;
  toggleMinimize: (id: string) => void;
  toggleMaximize: (id: string) => void;
}>((set, get) => ({
  windows: [],
  baseZIndex: 100,
  openApp: (appType, title, props = {}) => {
    if (!allowedAppTypes.has(appType)) return;
    const { windows, baseZIndex } = get();
    const id = `${appType}_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    const nextZ = baseZIndex + 1;
    const count = windows.length;
    const rect = placedWindowRect(appType, count, props?.placement);
    set({
      baseZIndex: nextZ,
      windows: [
        ...windows.map(window => ({ ...window, isFocused: false })),
        {
          instanceId: id,
          appType,
          title,
          zIndex: nextZ,
          isFocused: true,
          offsetX: rect.offsetX,
          offsetY: rect.offsetY,
          width: rect.width,
          height: rect.height,
          isMinimized: false,
          isMaximized: false,
          props,
        },
      ],
    });
  },
  closeWindow: (id) => set((state) => ({ windows: state.windows.filter(window => window.instanceId !== id) })),
  focusWindow: (id) => set((state) => {
    const window = state.windows.find(item => item.instanceId === id);
    if (window?.isFocused) return state;
    const nextZ = state.baseZIndex + 1;
    return {
      baseZIndex: nextZ,
      windows: state.windows.map(item => ({
        ...item,
        isFocused: item.instanceId === id,
        zIndex: item.instanceId === id ? nextZ : item.zIndex,
      })),
    };
  }),
  restoreWindow: (id) => set((state) => {
    const nextZ = state.baseZIndex + 1;
    return {
      baseZIndex: nextZ,
      windows: state.windows.map(item => ({
        ...item,
        isFocused: item.instanceId === id,
        isMinimized: item.instanceId === id ? false : item.isMinimized,
        zIndex: item.instanceId === id ? nextZ : item.zIndex,
      })),
    };
  }),
  updateWindowPosition: (id, x, y) => set((state) => {
    let changed: AppWindow | null = null;
    const windows = state.windows.map(window => {
      if (window.instanceId !== id || window.isMaximized) return window;
      const rect = clampWindowRect({ offsetX: x, offsetY: y, width: window.width, height: window.height }, window.appType);
      changed = { ...window, offsetX: rect.offsetX, offsetY: rect.offsetY };
      return changed;
    });
    if (changed) saveWindowLayout(changed);
    return { windows };
  }),
  updateWindowSize: (id, w, h) => set((state) => {
    let changed: AppWindow | null = null;
    const windows = state.windows.map(window => {
      if (window.instanceId !== id || window.isMaximized) return window;
      const rect = clampWindowRect({ offsetX: window.offsetX, offsetY: window.offsetY, width: w, height: h }, window.appType);
      changed = { ...window, ...rect };
      return changed;
    });
    if (changed) saveWindowLayout(changed);
    return { windows };
  }),
  toggleMinimize: (id) => set((state) => ({
    windows: state.windows.map(window => (
      window.instanceId === id ? { ...window, isMinimized: !window.isMinimized, isFocused: false } : window
    )),
  })),
  toggleMaximize: (id) => set((state) => ({
    windows: state.windows.map(item => {
      if (item.instanceId !== id) return item;
      const nextMaximized = !item.isMaximized;
      const maximized = maximizedWindowRect();
      const restored = clampWindowRect({
        offsetX: item.prevOffsetX ?? 80,
        offsetY: item.prevOffsetY ?? 80,
        width: item.prevWidth ?? 800,
        height: item.prevHeight ?? 600,
      }, item.appType);
      const nextWindow = {
        ...item,
        isMaximized: nextMaximized,
        isMinimized: false,
        prevOffsetX: nextMaximized ? item.offsetX : item.prevOffsetX,
        prevOffsetY: nextMaximized ? item.offsetY : item.prevOffsetY,
        prevWidth: nextMaximized ? item.width : item.prevWidth,
        prevHeight: nextMaximized ? item.height : item.prevHeight,
        offsetX: nextMaximized ? maximized.offsetX : restored.offsetX,
        offsetY: nextMaximized ? maximized.offsetY : restored.offsetY,
        width: nextMaximized ? maximized.width : restored.width,
        height: nextMaximized ? maximized.height : restored.height,
      };
      if (!nextMaximized) saveWindowLayout(nextWindow);
      return nextWindow;
    }),
  })),
}));

export const useChatStore = create<{
  sessions: Record<string, Session>;
  archivedSessions: Session[];
  currentSessionId: string;
  isThinking: boolean;
  thinkingBySession: Record<string, boolean>;
  tokens: { used: number; total: number };
  memories: MemoryItem[];
  hydrate: () => Promise<void>;
  addMessage: (msg: ChatMessage) => void;
  addLocalMessage: (msg: ChatMessage) => void;
  addLocalMessageToSession: (sessionId: string, msg: ChatMessage) => void;
  startStreamingMessage: (sessionId: string, messageId: string) => void;
  appendStreamingMessage: (sessionId: string, messageId: string, content: string) => void;
  completeStreamingMessage: (sessionId: string, messageId: string, content?: string) => void;
  failStreamingMessage: (sessionId: string, messageId: string, error: string) => void;
  setIsThinking: (t: boolean) => void;
  setSessionThinking: (sessionId: string, thinking: boolean) => void;
  createDetachedSession: (title: string, projectId?: string | null) => Promise<string>;
  ensureActiveSession: (title?: string) => Promise<string>;
  resetHydratedState: () => Promise<void>;
  removeSession: (id: string) => Promise<void>;
  archiveSession: (id: string) => Promise<void>;
  restoreSession: (id: string) => Promise<void>;
  deleteArchivedSession: (id: string) => Promise<void>;
  refreshArchivedSessions: () => Promise<void>;
  setSessionPinned: (id: string, pinned: boolean) => Promise<void>;
  addSession: (title: string, projectId?: string | null) => Promise<string>;
  renameSession: (id: string, title: string) => Promise<void>;
  addMemory: (text: string) => Promise<void>;
  removeMemory: (id: string) => Promise<void>;
  toggleMemory: (id: string) => Promise<void>;
  clearMemories: () => Promise<void>;
  loadSession: (id: string) => void;
}>((set, get) => {
  const cachedSessions = normalizeSessions(load('sessions', {}));
  const firstCached = Object.keys(cachedSessions)[0] || '';
  return {
    sessions: cachedSessions,
    archivedSessions: [],
    currentSessionId: load('currSess', firstCached),
    memories: normalizeMemories(load('memories', [])).map((memory, index) => ({
      id: `cached_${index}`,
      text: memory.text,
      enabled: memory.enabled,
      createdAt: Date.now(),
    })),
    isThinking: false,
    thinkingBySession: {},
    tokens: { used: 0, total: 0 },
    hydrate: async () => {
      if (!isTauriRuntime()) return;
      await initLocalDb();

      const oldMemories = normalizeMemories(load('memories', []));
      const oldSessions = normalizeSessions(load('sessions', {}));
      const existingSessions = await getSessions();
      const existingMemories = await getMemories();

      if (!existingMemories.length && oldMemories.length) {
        for (const memory of oldMemories) {
          const saved = await addMemoryDb(memory.text, 'localStorage_migration');
          if (!memory.enabled) await updateMemoryDb(saved.id, undefined, false);
        }
      }

      let sessions = existingSessions;
      if (!sessions.length && Object.keys(oldSessions).length) {
        for (const session of Object.values(oldSessions)) {
          const created = await createSessionDb(session.title || '旧会话');
          for (const message of session.messages || []) {
            await saveMessageDb(created.id, {
              id: message.id,
              role: roleToDb(message.role),
              content: message.content,
              created_at: message.timestamp,
              metadata: {},
            });
          }
        }
        sessions = await getSessions();
      }
      const sessionEntries: Array<[string, Session]> = [];
      for (const session of sessions) {
        const messages = (await getMessages(session.id)).map(mapMessage);
        sessionEntries.push([session.id, mapSession(session, messages)]);
      }

      const dbMemories = (await getMemories()).map(mapMemory);
      const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
      const current = get().currentSessionId;
      const currentSessionId = sessionEntries.some(([id]) => id === current) ? current : (sessionEntries[0]?.[0] || '');
      save('currSess', currentSessionId);
      set({
        sessions: Object.fromEntries(sessionEntries),
        archivedSessions: archivedEntries.map(([, session]) => session),
        currentSessionId,
        memories: dbMemories,
      });
    },
    addMessage: (msg) => set((state) => {
      const session = state.sessions[state.currentSessionId];
      if (!session) return state;
      if (isDuplicateMessage(session.messages, msg)) return state;
      const nextSessions = {
        ...state.sessions,
        [state.currentSessionId]: {
          ...session,
          messages: [...session.messages, msg],
          time: new Date(msg.timestamp).toLocaleString(),
        },
      };
      if (isTauriRuntime()) {
        saveMessageDb(state.currentSessionId, {
          id: msg.id,
          role: roleToDb(msg.role),
          content: msg.content,
          created_at: msg.timestamp,
          metadata: {},
        }).catch((error) => EventBus.dispatch('TOAST', { msg: '消息保存失败：' + error, type: 'warning' }));
      }
      return {
        sessions: nextSessions,
        tokens: {
          used: state.tokens.used + Math.ceil(msg.content.length / 2),
          total: state.tokens.total,
        },
      };
    }),
    addLocalMessage: (msg) => set((state) => {
      const session = state.sessions[state.currentSessionId];
      if (!session) return state;
      if (isDuplicateMessage(session.messages, msg)) return state;
      const shouldTitleFromUser =
        msg.role === 'user' &&
        !session.titleIsManual &&
        (session.title === '新会话' || session.title === 'New session' || session.title === 'Initial session') &&
        session.messages.filter(message => message.role === 'user').length === 0;
      return {
        sessions: {
          ...state.sessions,
          [state.currentSessionId]: {
            ...session,
            title: shouldTitleFromUser ? titleFromMessage(msg.content) : session.title,
            messages: [...session.messages, msg],
            time: new Date(msg.timestamp).toLocaleString(),
          },
        },
        tokens: {
          used: state.tokens.used + Math.ceil(msg.content.length / 2),
          total: state.tokens.total,
        },
      };
    }),
    addLocalMessageToSession: (sessionId, msg) => set((state) => {
      const session = state.sessions[sessionId];
      if (!session) return state;
      if (isDuplicateMessage(session.messages, msg)) return state;
      const shouldTitleFromUser =
        msg.role === 'user' &&
        !session.titleIsManual &&
        (session.title === '新会话' || session.title === 'New session' || session.title === 'Initial session') &&
        session.messages.filter(message => message.role === 'user').length === 0;
      return {
        sessions: {
          ...state.sessions,
          [sessionId]: {
            ...session,
            title: shouldTitleFromUser ? titleFromMessage(msg.content) : session.title,
            messages: [...session.messages, msg],
            time: new Date(msg.timestamp).toLocaleString(),
          },
        },
        tokens: {
          used: state.tokens.used + Math.ceil(msg.content.length / 2),
          total: state.tokens.total,
        },
      };
    }),
    startStreamingMessage: (sessionId, messageId) => set((state) => {
      const session = state.sessions[sessionId];
      if (!session) return state;
      if (session.messages.some(message => message.id === messageId)) return state;
      const msg: ChatMessage = {
        id: messageId,
        role: 'aura',
        content: '',
        timestamp: Date.now(),
        streaming: true,
      };
      return {
        sessions: {
          ...state.sessions,
          [sessionId]: {
            ...session,
            messages: [...session.messages, msg],
            time: new Date(msg.timestamp).toLocaleString(),
          },
        },
      };
    }),
    appendStreamingMessage: (sessionId, messageId, content) => set((state) => {
      const session = state.sessions[sessionId];
      if (!session) return state;
      const hasMessage = session.messages.some(message => message.id === messageId);
      const messages = hasMessage
        ? session.messages.map(message => message.id === messageId ? { ...message, content: `${message.content}${content}`, streaming: true } : message)
        : [...session.messages, { id: messageId, role: 'aura' as const, content, timestamp: Date.now(), streaming: true }];
      return {
        sessions: {
          ...state.sessions,
          [sessionId]: { ...session, messages, time: new Date().toLocaleString() },
        },
      };
    }),
    completeStreamingMessage: (sessionId, messageId, content) => set((state) => {
      const session = state.sessions[sessionId];
      if (!session) return state;
      const existing = session.messages.find(message => message.id === messageId);
      const finalContent = content ?? existing?.content ?? '';
      const messages = existing
        ? finalContent.trim()
          ? session.messages.map(message => message.id === messageId ? { ...message, content: finalContent, streaming: false, timestamp: message.timestamp || Date.now() } : message)
          : session.messages.filter(message => message.id !== messageId)
        : finalContent.trim()
          ? [...session.messages, { id: messageId, role: 'aura' as const, content: finalContent, timestamp: Date.now(), streaming: false }]
          : session.messages;
      return {
        sessions: {
          ...state.sessions,
          [sessionId]: { ...session, messages, time: new Date().toLocaleString() },
        },
      };
    }),
    failStreamingMessage: (sessionId, messageId, error) => set((state) => {
      const session = state.sessions[sessionId];
      if (!session) return state;
      const content = error || 'Aura 回复中断。';
      const messages = session.messages.some(message => message.id === messageId)
        ? session.messages.map(message => message.id === messageId ? { ...message, content: message.content || content, streaming: false, error: true } : message)
        : [...session.messages, { id: messageId, role: 'system' as const, content, timestamp: Date.now(), error: true }];
      return {
        sessions: {
          ...state.sessions,
          [sessionId]: { ...session, messages, time: new Date().toLocaleString() },
        },
      };
    }),
    setIsThinking: (isThinking) => set({ isThinking }),
    setSessionThinking: (sessionId, thinking) => set((state) => ({
      isThinking: sessionId === state.currentSessionId ? thinking : state.isThinking,
      thinkingBySession: { ...state.thinkingBySession, [sessionId]: thinking },
    })),
    createDetachedSession: async (title, projectId = null) => {
      if (isTauriRuntime()) {
        const record = await createSessionDb(title || '新会话', projectId);
        const session = mapSession(record);
        logAuraActivity('agent', '新建独立窗口会话', session.title, { sessionId: session.id }).catch(() => {});
        set((state) => ({
          sessions: { ...state.sessions, [session.id]: session },
          thinkingBySession: { ...state.thinkingBySession, [session.id]: false },
        }));
        return session.id;
      }
      const id = `s_${Date.now()}_${Math.random().toString(36).slice(2)}`;
      set((state) => ({
        sessions: { ...state.sessions, [id]: { id, title, time: '刚刚', messages: [], projectId } },
        thinkingBySession: { ...state.thinkingBySession, [id]: false },
      }));
      return id;
    },
    ensureActiveSession: async (title = '新会话') => {
      const state = get();
      if (state.currentSessionId && state.sessions[state.currentSessionId]) {
        return state.currentSessionId;
      }

      const firstExistingId = Object.keys(state.sessions)[0];
      if (firstExistingId) {
        save('currSess', firstExistingId);
        set({ currentSessionId: firstExistingId });
        return firstExistingId;
      }

      if (isTauriRuntime()) {
        const record = await createSessionDb(title || '新会话');
        const session = mapSession(record);
        save('currSess', session.id);
        set((state) => ({
          sessions: { ...state.sessions, [session.id]: session },
          currentSessionId: session.id,
          thinkingBySession: { ...state.thinkingBySession, [session.id]: false },
          tokens: { used: 0, total: 0 },
        }));
        return session.id;
      }

      const id = `s_${Date.now()}_${Math.random().toString(36).slice(2)}`;
      save('currSess', id);
      set((state) => ({
        sessions: { ...state.sessions, [id]: { id, title: title || '新会话', time: '刚刚', messages: [] } },
        currentSessionId: id,
        thinkingBySession: { ...state.thinkingBySession, [id]: false },
        tokens: { used: 0, total: 0 },
      }));
      return id;
    },
    resetHydratedState: async () => {
      if (!isTauriRuntime()) return;
      const [records, memories] = await Promise.all([
        getSessions().catch(() => [] as SessionRecord[]),
        getMemories().catch(() => [] as MemoryRecord[]),
      ]);
      const usableRecords = records.length ? records : [await createSessionDb('新会话')];
      const sessionEntries = await mapSessionRecords(usableRecords);
      const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
      const currentSessionId = sessionEntries[0]?.[0] || '';
      save('currSess', currentSessionId);
      set({
        sessions: Object.fromEntries(sessionEntries),
        archivedSessions: archivedEntries.map(([, session]) => session),
        currentSessionId,
        memories: memories.map(mapMemory),
        isThinking: false,
        thinkingBySession: {},
      });
    },
    removeSession: async (id) => {
      if (isTauriRuntime()) {
        try {
          let records = await deleteSessionDb(id);
          if (!records.length) records = [await createSessionDb('新会话')];
          const entries = await mapSessionRecords(records);
          const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
          const currentSessionId = entries[0]?.[0] || '';
          save('currSess', currentSessionId);
          set({ sessions: Object.fromEntries(entries), archivedSessions: archivedEntries.map(([, session]) => session), currentSessionId });
        } catch (error) {
          EventBus.dispatch('TOAST', { msg: '删除会话失败：' + error, type: 'warning' });
          throw error;
        }
        return;
      }
      set((state) => {
        const nextSessions = { ...state.sessions };
        delete nextSessions[id];
        if (!Object.keys(nextSessions).length) {
          const replacementId = `s_${Date.now()}`;
          nextSessions[replacementId] = { id: replacementId, title: '新会话', time: '刚刚', messages: [] };
        }
        const nextCurrentId = state.currentSessionId === id ? Object.keys(nextSessions)[0] : state.currentSessionId;
        return { sessions: nextSessions, currentSessionId: nextCurrentId };
      });
    },
    archiveSession: async (id) => {
      if (isTauriRuntime()) {
        const records = await archiveSessionDb(id);
        const entries = await mapSessionRecords(records);
        const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
        const current = get().currentSessionId;
        const currentSessionId = entries.some(([sessionId]) => sessionId === current) ? current : (entries[0]?.[0] || '');
        save('currSess', currentSessionId);
        set({ sessions: Object.fromEntries(entries), archivedSessions: archivedEntries.map(([, session]) => session), currentSessionId });
        return;
      }
      set((state) => {
        const session = state.sessions[id];
        if (!session) return state;
        const nextSessions = { ...state.sessions };
        delete nextSessions[id];
        const archived = [{ ...session, archivedAt: Date.now(), pinned: false }, ...state.archivedSessions];
        if (!Object.keys(nextSessions).length) {
          const replacementId = `s_${Date.now()}`;
          nextSessions[replacementId] = { id: replacementId, title: '新会话', time: '刚刚', messages: [], createdAt: Date.now() };
        }
        const currentSessionId = state.currentSessionId === id ? Object.keys(nextSessions)[0] : state.currentSessionId;
        return { sessions: nextSessions, archivedSessions: archived, currentSessionId };
      });
    },
    restoreSession: async (id) => {
      if (isTauriRuntime()) {
        const record = await restoreSessionDb(id);
        const session = mapSession(record, (await getMessages(record.id).catch(() => [] as MessageRecord[])).map(mapMessage));
        const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
        set((state) => ({
          sessions: { ...state.sessions, [session.id]: session },
          archivedSessions: archivedEntries.map(([, archived]) => archived),
        }));
        return;
      }
      set((state) => {
        const session = state.archivedSessions.find(item => item.id === id);
        if (!session) return state;
        return {
          sessions: { ...state.sessions, [id]: { ...session, archivedAt: null } },
          archivedSessions: state.archivedSessions.filter(item => item.id !== id),
        };
      });
    },
    deleteArchivedSession: async (id) => {
      if (isTauriRuntime()) {
        const records = await deleteSessionDb(id);
        const entries = await mapSessionRecords(records);
        const archivedEntries = await mapSessionRecords(await getArchivedSessions().catch(() => [] as SessionRecord[]));
        const current = get().currentSessionId;
        const currentSessionId = entries.some(([sessionId]) => sessionId === current) ? current : (entries[0]?.[0] || '');
        save('currSess', currentSessionId);
        set({ sessions: Object.fromEntries(entries), archivedSessions: archivedEntries.map(([, session]) => session), currentSessionId });
        return;
      }
      set((state) => ({ archivedSessions: state.archivedSessions.filter(session => session.id !== id) }));
    },
    refreshArchivedSessions: async () => {
      if (!isTauriRuntime()) return;
      const archivedEntries = await mapSessionRecords(await getArchivedSessions());
      set({ archivedSessions: archivedEntries.map(([, session]) => session) });
    },
    setSessionPinned: async (id, pinned) => {
      if (isTauriRuntime()) {
        const record = await setSessionPinnedDb(id, pinned);
        const currentMessages = get().sessions[id]?.messages || [];
        set((state) => ({
          sessions: {
            ...state.sessions,
            [id]: mapSession(record, currentMessages),
          },
        }));
        return;
      }
      set((state) => {
        const session = state.sessions[id];
        if (!session) return state;
        return { sessions: { ...state.sessions, [id]: { ...session, pinned } } };
      });
    },
    addSession: async (title, projectId = null) => {
      if (isTauriRuntime()) {
        try {
          const record = await createSessionDb(title || '新会话', projectId);
          const session = mapSession(record);
          save('currSess', session.id);
          logAuraActivity('agent', '新建会话', session.title, { sessionId: session.id }).catch(() => {});
          set((state) => ({
            sessions: { ...state.sessions, [session.id]: session },
            currentSessionId: session.id,
            tokens: { used: 0, total: 0 },
          }));
          return session.id;
        } catch (error) {
          EventBus.dispatch('TOAST', { msg: `新建会话失败：${getErrorMessage(error)}`, type: 'warning' });
          throw error;
        }
      }
      const id = `s_${Date.now()}`;
      set((state) => ({
        sessions: { ...state.sessions, [id]: { id, title, time: '刚刚', messages: [], projectId } },
        currentSessionId: id,
        tokens: { used: 0, total: 0 },
      }));
      return id;
    },
    renameSession: async (id, title) => {
      const trimmed = title.trim();
      if (!trimmed) return;
      if (isTauriRuntime()) {
        const record = await renameSessionDb(id, trimmed);
        const currentMessages = get().sessions[id]?.messages || [];
        set((state) => ({
          sessions: {
            ...state.sessions,
            [id]: mapSession(record, currentMessages),
          },
        }));
        return;
      }
      set((state) => {
        const session = state.sessions[id];
        if (!session) return state;
        return {
          sessions: {
            ...state.sessions,
            [id]: { ...session, title: trimmed, titleIsManual: true },
          },
        };
      });
    },
    addMemory: async (text) => {
      const trimmed = text.trim();
      if (!trimmed) return;
      const duplicate = get().memories.find(memory => memory.text.trim().toLowerCase() === trimmed.toLowerCase());
      if (duplicate) {
        EventBus.dispatch('TOAST', {
          msg: duplicate.enabled ? '这条记忆已经存在。' : '这条记忆已经存在但当前停用，可在记忆页重新启用。',
          type: 'info',
        });
        return;
      }
      if (isTauriRuntime()) {
        const record = await addMemoryDb(trimmed, 'manual');
        logAuraActivity('memory', '新增长期记忆', trimmed, { id: record.id }).catch(() => {});
        set((state) => ({ memories: [mapMemory(record), ...state.memories] }));
        return;
      }
      set((state) => ({ memories: [{ id: `m_${Date.now()}`, text: trimmed, createdAt: Date.now(), enabled: true }, ...state.memories] }));
    },
    removeMemory: async (id) => {
      if (isTauriRuntime()) await deleteMemoryDb(id);
      set((state) => ({ memories: state.memories.filter(memory => memory.id !== id) }));
    },
    toggleMemory: async (id) => {
      const memory = get().memories.find(item => item.id === id);
      if (!memory) return;
      if (isTauriRuntime()) {
        const nextEnabled = !memory.enabled;
        const record = await updateMemoryDb(id, undefined, nextEnabled);
        logAuraActivity('memory', nextEnabled ? '启用长期记忆' : '停用长期记忆', memory.text, { id }).catch(() => {});
        set((state) => ({
          memories: state.memories.map(item => item.id === id ? mapMemory(record) : item),
        }));
        return;
      }
      set((state) => ({
        memories: state.memories.map(memory => memory.id === id ? { ...memory, enabled: !memory.enabled } : memory),
      }));
    },
    clearMemories: async () => {
      if (isTauriRuntime()) await clearMemoriesDb();
      set({ memories: [] });
    },
    loadSession: (id) => {
      save('currSess', id);
      set({ currentSessionId: id });
    },
  };
});

export async function hydrateLocalData() {
  if (!isTauriRuntime()) return;
  await initLocalDb();
  await Promise.all([
    useAuthStore.getState().hydrateProfile(),
    useChatStore.getState().hydrate(),
  ]);
}
