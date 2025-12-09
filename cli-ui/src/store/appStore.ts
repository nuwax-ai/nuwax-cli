import { create } from 'zustand';
import { DEFAULT_LOG_CONFIG, LogConfig, LogEntry } from '../types';

export type ValidationState = 'validating' | 'valid' | 'invalid' | 'none';

export interface WorkingDirectoryStore {
  path: string | null;
  isValid: boolean;
  validationState: ValidationState;
  error?: string;
}

export interface AppStore {
  workingDirectory: WorkingDirectoryStore;
  showWelcomeModal: boolean;
  isAppLoading: boolean;
  isInitialized: boolean;
  isExecuting: boolean;
  logs: LogEntry[];
  totalLogCount: number;
  logConfig: LogConfig;
  setWorkingDirectory: (payload: Partial<WorkingDirectoryStore>) => void;
  setValidationState: (state: ValidationState, error?: string) => void;
  setShowWelcomeModal: (show: boolean) => void;
  setAppLoading: (flag: boolean) => void;
  setInitialized: (flag: boolean) => void;
  setExecuting: (flag: boolean) => void;
  addLog: (
    type: LogEntry['type'],
    message: string,
    command?: string,
    args?: string[]
  ) => void;
  appendLogs: (entries: LogEntry[]) => void;
  clearLogs: () => void;
}

const generateLogEntry = (
  type: LogEntry['type'],
  message: string,
  command?: string,
  args?: string[]
): LogEntry => ({
  id: `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`,
  timestamp: new Date().toLocaleTimeString(),
  type,
  message,
  command,
  args,
});

export const useAppStore = create<AppStore>((set, get) => ({
  workingDirectory: {
    path: null,
    isValid: false,
    validationState: 'none',
    error: undefined,
  },
  showWelcomeModal: false,
  isAppLoading: true,
  isInitialized: false,
  isExecuting: false,
  logs: [],
  totalLogCount: 0,
  logConfig: DEFAULT_LOG_CONFIG,

  setWorkingDirectory: (payload) =>
    set((state) => ({
      workingDirectory: {
        ...state.workingDirectory,
        ...payload,
      },
    })),

  setValidationState: (validationState, error) =>
    set((state) => ({
      workingDirectory: {
        ...state.workingDirectory,
        validationState,
        error,
      },
    })),

  setShowWelcomeModal: (show) => set(() => ({ showWelcomeModal: show })),
  setAppLoading: (flag) => set(() => ({ isAppLoading: flag })),
  setInitialized: (flag) => set(() => ({ isInitialized: flag })),
  setExecuting: (flag) => set(() => ({ isExecuting: flag })),

  addLog: (type, message, command, args) => {
    if (!message.trim() && type !== 'command') return;
    const entry = generateLogEntry(type, message, command, args);
    const { logConfig } = get();

    set((state) => {
      const merged = [...state.logs];
      const last = merged[merged.length - 1];

      // 轻量去重：跳过连续相同的消息和类型
      if (last && last.message === entry.message && last.type === entry.type) {
        return {};
      }

      merged.push(entry);
      let totalLogCount = state.totalLogCount + 1;

      if (merged.length > logConfig.maxEntries) {
        const excessCount = merged.length - logConfig.maxEntries;
        const trimCount = Math.max(excessCount, logConfig.trimBatchSize);
        merged.splice(0, trimCount);
      }

      return {
        logs: merged,
        totalLogCount,
      };
    });
  },

  appendLogs: (entries) => {
    if (!entries.length) return;
    const { logConfig } = get();

    set((state) => {
      const merged = [...state.logs];
      let added = 0;

      for (const entry of entries) {
        const last = merged[merged.length - 1];
        if (last && last.message === entry.message && last.type === entry.type) {
          continue;
        }
        merged.push(entry);
        added += 1;
      }

      if (!added) {
        return {};
      }

      if (merged.length > logConfig.maxEntries) {
        const excessCount = merged.length - logConfig.maxEntries;
        const trimCount = Math.max(excessCount, logConfig.trimBatchSize);
        merged.splice(0, trimCount);
      }

      return {
        logs: merged,
        totalLogCount: state.totalLogCount + added,
      };
    });
  },

  clearLogs: () =>
    set(() => ({
      logs: [],
      totalLogCount: 0,
    })),
}));

