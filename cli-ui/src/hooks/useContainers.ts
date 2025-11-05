import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ContainerInfo } from '../types';
import { useApp } from '../context/AppContext';

interface UseContainersReturn {
  containers: ContainerInfo[];
  isLoading: boolean;
  error: string | null;
  refreshContainers: () => Promise<void>;
}

export const useContainers = (autoRefresh: boolean = false, refreshInterval: number = 5000): UseContainersReturn => {
  const { workingDirectory, setContainers, containers: contextContainers } = useApp();
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  const refreshContainers = useCallback(async () => {
    if (!workingDirectory) {
      setError('工作目录未设置');
      return;
    }

    setIsLoading(true);
    setError(null);

    try {
      const result = await invoke<ContainerInfo[]>('get_container_status', {
        workingDirectory,
      });
      setContainers(result);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      console.error('Failed to fetch container status:', err);
    } finally {
      setIsLoading(false);
    }
  }, [workingDirectory, setContainers]);

  // 自动刷新
  useEffect(() => {
    if (autoRefresh && workingDirectory) {
      refreshContainers();
      const intervalId = setInterval(refreshContainers, refreshInterval);
      return () => clearInterval(intervalId);
    }
  }, [autoRefresh, refreshInterval, workingDirectory, refreshContainers]);

  return {
    containers: contextContainers,
    isLoading,
    error,
    refreshContainers,
  };
};
