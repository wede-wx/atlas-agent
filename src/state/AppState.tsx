import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import {
  createSession,
  getSessions,
  getUiPreferences,
  isTauri,
  listProjects,
  saveUiPreference,
} from "../bridge";
import type { ProjectRecord, SessionRecord, UiPreferences } from "../types";

const defaultPrefs: UiPreferences = {
  theme: { mode: "dark" },
  notifications: { runCompleted: true, blockedGate: true, permissionNeeded: true, sound: false },
  general: { defaultAgentMode: "chat", autoCreateSession: true, openDrawerOnRun: true },
  layout: { sidebarCollapsed: false, rightDrawerOpen: true, rightDrawerTab: "contract" },
};

interface AppStateValue {
  tauriReady: boolean;
  sessions: SessionRecord[];
  projects: ProjectRecord[];
  activeSessionId: string | null;
  prefs: UiPreferences;
  loading: boolean;
  error: string | null;
  setActiveSessionId: (id: string | null) => void;
  refreshSessions: () => Promise<void>;
  refreshProjects: () => Promise<void>;
  createNewSession: (title?: string, projectId?: string | null) => Promise<SessionRecord | null>;
  updatePreference: <K extends keyof UiPreferences>(key: K, value: UiPreferences[K]) => Promise<void>;
}

const AppStateContext = createContext<AppStateValue | null>(null);

export function AppProvider({ children }: { children: React.ReactNode }) {
  const [tauriReady] = useState(isTauri());
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [projects, setProjects] = useState<ProjectRecord[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [prefs, setPrefs] = useState<UiPreferences>(defaultPrefs);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refreshSessions = useCallback(async () => {
    if (!tauriReady) return;
    const rows = await getSessions();
    setSessions(rows);
    setActiveSessionId((current) => current ?? rows[0]?.id ?? null);
  }, [tauriReady]);

  const refreshProjects = useCallback(async () => {
    if (!tauriReady) return;
    const rows = await listProjects();
    setProjects(rows);
  }, [tauriReady]);

  useEffect(() => {
    let mounted = true;
    async function bootstrap() {
      setLoading(true);
      setError(null);
      if (!tauriReady) {
        setLoading(false);
        setError("当前不在 Tauri 运行时中，真实后端 command 不可用。");
        return;
      }
      try {
        const loadedPrefs = await getUiPreferences(defaultPrefs);
        if (!mounted) return;
        setPrefs(loadedPrefs);
        document.documentElement.dataset.theme = loadedPrefs.theme.mode === "system" ? "dark" : loadedPrefs.theme.mode;
        await Promise.all([refreshSessions(), refreshProjects()]);
      } catch (err) {
        if (mounted) setError(String(err));
      } finally {
        if (mounted) setLoading(false);
      }
    }
    void bootstrap();
    return () => {
      mounted = false;
    };
  }, [refreshProjects, refreshSessions, tauriReady]);

  const createNewSession = useCallback(async (title = "New session", projectId?: string | null) => {
    if (!tauriReady) return null;
    const session = await createSession(title, projectId ?? null);
    setSessions((rows) => [session, ...rows.filter((row) => row.id !== session.id)]);
    setActiveSessionId(session.id);
    return session;
  }, [tauriReady]);

  const updatePreference = useCallback(async <K extends keyof UiPreferences>(key: K, value: UiPreferences[K]) => {
    setPrefs((prev) => {
      const next = { ...prev, [key]: value };
      if (key === "theme") {
        const themeValue = value as UiPreferences["theme"];
        document.documentElement.dataset.theme = themeValue.mode === "system" ? "dark" : themeValue.mode;
      }
      return next;
    });
    if (tauriReady) await saveUiPreference(key, value);
  }, [tauriReady]);

  const value = useMemo<AppStateValue>(() => ({
    tauriReady,
    sessions,
    projects,
    activeSessionId,
    prefs,
    loading,
    error,
    setActiveSessionId,
    refreshSessions,
    refreshProjects,
    createNewSession,
    updatePreference,
  }), [activeSessionId, createNewSession, error, loading, prefs, projects, refreshProjects, refreshSessions, sessions, tauriReady, updatePreference]);

  return <AppStateContext.Provider value={value}>{children}</AppStateContext.Provider>;
}

export function useAppState(): AppStateValue {
  const value = useContext(AppStateContext);
  if (!value) throw new Error("useAppState must be used inside AppProvider");
  return value;
}


