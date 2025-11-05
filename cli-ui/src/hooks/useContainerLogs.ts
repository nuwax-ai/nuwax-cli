import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useApp } from '../context/AppContext';

interface UseContainerLogsReturn {
  isStreaming: boolean;
  error: string | null;
  startStreaming: (containerName: string) => Promise<void>;
  stopStreaming: () => Promise<void>;
}

interface ContainerLogEvent {
  container_name: string;
  log: string;
}

export const useContainerLogs = (): UseContainerLogsReturn => {
  const { workingDirectory, addLog } = useApp();
  const [isStreaming, setIsStreaming] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [currentContainer, setCurrentContainer] = useState<string | null>(null);
  const [unlistenFn, setUnlistenFn] = useState<UnlistenFn | null>(null);

  // 开始流式传输容器日志
  const startStreaming = useCallback(async (containerName: string) => {
    if (!workingDirectory) {
      setError('工作目录未设置');
      return;
    }

    // 如果已经在流式传输，先停止
    if (isStreaming && currentContainer) {
      await stopStreaming();
    }

    setIsStreaming(true);
    setError(null);
    setCurrentContainer(containerName);

    try {
      // 监听容器日志事件
      const unlisten = await listen<ContainerLogEvent>('container-log', (event) => {
        if (event.payload.container_name === containerName) {
          addLog({
            type: 'info',
            message: event.payload.log,
          });
        }
      });
      setUnlistenFn(() => unlisten);

      // 监听日志流完成事件
      const unlistenComplete = await listen('container-log-complete', () => {
        setIsStreaming(false);
        setCurrentContainer(null);
      });

      // 调用 Tauri 命令开始流式传输
      await invoke('stream_container_logs', {
        workingDirectory,
        containerName,
        follow: true,
      });

      // 清理完成监听器
      return () => {
        unlistenComplete();
      };
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      setIsStreaming(false);
      setCurrentContainer(null);
      console.error('Failed to start container log streaming:', err);
    }
  }, [workingDirectory, isStreaming, currentContainer, addLog]);

  // 停止流式传输
  const stopStreaming = useCallback(async () => {
    if (!currentContainer) {
      return;
    }

    try {
      await invoke('stop_container_logs', {
        containerName: currentContainer,
      });

      // 取消事件监听
      if (unlistenFn) {
        unlistenFn();
        setUnlistenFn(null);
      }

      setIsStreaming(false);
      setCurrentContainer(null);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      console.error('Failed to stop container log streaming:', err);
    }
  }, [currentContainer, unlistenFn]);

  // 组件卸载时停止流式传输
  useEffect(() => {
    return () => {
      if (isStreaming && currentContainer) {
        stopStreaming();
      }
    };
  }, []);

  return {
    isStreaming,
    error,
    startStreaming,
    stopStreaming,
  };
};
