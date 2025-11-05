import React, { createContext, useContext, useState, useCallback, useEffect } from 'react';
import { LogEntry, LogConfig, DEFAULT_LOG_CONFIG, ContainerInfo } from '../types';

interface AppContextType {
  // 工作目录
  workingDirectory: string | null;
  isDirectoryValid: boolean;
  setWorkingDirectory: (dir: string | null, valid: boolean) => void;
  
  // 全局日志
  logs: LogEntry[];
  addLog: (log: Omit<LogEntry, 'id' | 'timestamp'>) => void;
  clearLogs: () => void;
  
  // 容器状态
  containers: ContainerInfo[];
  setContainers: (containers: ContainerInfo[]) => void;
  refreshContainers: () => Promise<void>;
  
  // 执行状态
  isExecuting: boolean;
  setIsExecuting: (executing: boolean) => void;
  
  // 日志配置
  logConfig: LogConfig;
}

const AppContext = createContext<AppContextType | undefined>(undefined);

interface AppProviderProps {
  children: React.ReactNode;
}

export const AppProvider: React.FC<AppProviderProps> = ({ children }) => {
  const [workingDirectory, setWorkingDirectoryState] = useState<string | null>(null);
  const [isDirectoryValid, setIsDirectoryValid] = useState<boolean>(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [containers, setContainers] = useState<ContainerInfo[]>([]);
  const [isExecuting, setIsExecuting] = useState<boolean>(false);
  const [logConfig] = useState<LogConfig>(DEFAULT_LOG_CONFIG);

  // 设置工作目录
  const setWorkingDirectory = useCallback((dir: string | null, valid: boolean) => {
    setWorkingDirectoryState(dir);
    setIsDirectoryValid(valid);
  }, []);

  // 添加日志（带循环缓冲区逻辑）
  const addLog = useCallback((logData: Omit<LogEntry, 'id' | 'timestamp'>) => {
    setLogs((prevLogs) => {
      const newLog: LogEntry = {
        ...logData,
        id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
        timestamp: new Date().toISOString(),
      };

      const updatedLogs = [...prevLogs, newLog];

      // 循环缓冲区：如果超过最大条目数，删除最旧的日志
      if (updatedLogs.length > logConfig.maxEntries) {
        const trimCount = logConfig.trimBatchSize;
        return updatedLogs.slice(trimCount);
      }

      return updatedLogs;
    });
  }, [logConfig]);

  // 清除日志
  const clearLogs = useCallback(() => {
    setLogs([]);
  }, []);

  // 刷新容器状态（占位符，实际实现在 useContainers hook 中）
  const refreshContainers = useCallback(async () => {
    // 这个方法会在 useContainers hook 中被覆盖
    console.log('refreshContainers called from context');
  }, []);

  const contextValue: AppContextType = {
    workingDirectory,
    isDirectoryValid,
    setWorkingDirectory,
    logs,
    addLog,
    clearLogs,
    containers,
    setContainers,
    refreshContainers,
    isExecuting,
    setIsExecuting,
    logConfig,
  };

  return (
    <AppContext.Provider value={contextValue}>
      {children}
    </AppContext.Provider>
  );
};

// 自定义 Hook
export const useApp = (): AppContextType => {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error('useApp must be used within AppProvider');
  }
  return context;
};
