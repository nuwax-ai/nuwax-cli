import React, { useState, useEffect } from 'react';
import { useApp } from '../context/AppContext';
import { useContainers } from '../hooks/useContainers';
import { ContainerCard } from '../components/ContainerCard';
import { LogTerminal } from '../components/LogTerminal';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { LogEntry } from '../types';
import {
  ArrowPathIcon,
  XMarkIcon,
} from '@heroicons/react/24/outline';

export const ContainersPage: React.FC = () => {
  const { workingDirectory, isDirectoryValid, addLog } = useApp();
  const { containers, isLoading, error, refreshContainers } = useContainers(true, 5000);
  const [isOperating, setIsOperating] = useState(false);
  const [selectedContainer, setSelectedContainer] = useState<string | null>(null);
  const [containerLogs, setContainerLogs] = useState<LogEntry[]>([]);
  const [isStreamingLogs, setIsStreamingLogs] = useState(false);

  // 监听容器日志事件
  useEffect(() => {
    const unlistenLog = listen<string>('container-log', (event) => {
      const logEntry: LogEntry = {
        id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
        timestamp: new Date().toISOString(),
        type: 'info',
        message: event.payload,
      };
      setContainerLogs((prev) => [...prev, logEntry]);
    });

    const unlistenComplete = listen<string>('container-log-complete', (event) => {
      const containerName = event.payload;
      if (containerName === selectedContainer) {
        setIsStreamingLogs(false);
        const logEntry: LogEntry = {
          id: `${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
          timestamp: new Date().toISOString(),
          type: 'info',
          message: `--- 日志流结束 ---`,
        };
        setContainerLogs((prev) => [...prev, logEntry]);
      }
    });

    return () => {
      unlistenLog.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
    };
  }, [selectedContainer]);

  // 查看容器日志
  const handleViewLogs = async (containerName: string) => {
    // 如果已经在查看该容器的日志，则关闭
    if (selectedContainer === containerName) {
      if (isStreamingLogs) {
        try {
          await invoke('stop_container_logs', { containerName });
        } catch (err) {
          console.error('停止日志流失败:', err);
        }
      }
      setSelectedContainer(null);
      setContainerLogs([]);
      setIsStreamingLogs(false);
      return;
    }

    // 停止之前的日志流
    if (selectedContainer && isStreamingLogs) {
      try {
        await invoke('stop_container_logs', { containerName: selectedContainer });
      } catch (err) {
        console.error('停止之前的日志流失败:', err);
      }
    }

    // 开始新的日志流
    setSelectedContainer(containerName);
    setContainerLogs([]);
    setIsStreamingLogs(true);

    try {
      addLog({
        type: 'info',
        message: `开始查看容器日志: ${containerName}`,
      });

      await invoke('stream_container_logs', {
        containerName,
        follow: true,
      });
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `查看容器日志失败: ${errorMessage}`,
      });
      setIsStreamingLogs(false);
      setSelectedContainer(null);
    }
  };

  // 关闭日志查看器
  const handleCloseLogs = async () => {
    if (selectedContainer && isStreamingLogs) {
      try {
        await invoke('stop_container_logs', { containerName: selectedContainer });
      } catch (err) {
        console.error('停止日志流失败:', err);
      }
    }
    setSelectedContainer(null);
    setContainerLogs([]);
    setIsStreamingLogs(false);
  };

  // 启动容器
  const handleStartContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('start_container', {
        containerName,
      });

      addLog({
        type: 'success',
        message: `容器 ${containerName} 启动成功`,
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `启动容器失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
    }
  };

  // 停止容器
  const handleStopContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('stop_container', {
        containerName,
      });

      addLog({
        type: 'success',
        message: `容器 ${containerName} 停止成功`,
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `停止容器失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
    }
  };

  // 重启容器
  const handleRestartContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('restart_container', {
        containerName,
      });

      addLog({
        type: 'success',
        message: `容器 ${containerName} 重启成功`,
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `重启容器失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
    }
  };

  if (!workingDirectory || !isDirectoryValid) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center">
          <p className="text-gray-500 text-lg mb-4">请先设置有效的工作目录</p>
          <p className="text-gray-400 text-sm">
            点击顶部的"更改"按钮选择工作目录
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      {/* 容器列表 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold text-gray-900">容器列表</h2>
          <button
            onClick={refreshContainers}
            disabled={isLoading}
            className="flex items-center px-3 py-1.5 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <ArrowPathIcon className={`w-4 h-4 mr-1 ${isLoading ? 'animate-spin' : ''}`} />
            刷新
          </button>
        </div>

        {error && (
          <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-lg">
            <p className="text-red-600 text-sm">{error}</p>
          </div>
        )}

        {isLoading && containers.length === 0 ? (
          <div className="text-center py-8">
            <div className="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
            <p className="text-gray-500 mt-2">加载容器状态...</p>
          </div>
        ) : containers.length === 0 ? (
          <div className="text-center py-8">
            <p className="text-gray-500">暂无容器</p>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {containers.map((container) => (
              <div
                key={container.name}
                className={`${
                  selectedContainer === container.name
                    ? 'ring-2 ring-blue-500'
                    : ''
                }`}
              >
                <ContainerCard
                  container={container}
                  onViewLogs={handleViewLogs}
                  onStart={handleStartContainer}
                  onStop={handleStopContainer}
                  onRestart={handleRestartContainer}
                  isLoading={isOperating}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      {/* 容器日志查看器 */}
      {selectedContainer && (
        <div className="bg-white rounded-lg border border-gray-200 p-6">
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center space-x-3">
              <h2 className="text-xl font-semibold text-gray-900">
                容器日志: {selectedContainer}
              </h2>
              {isStreamingLogs && (
                <span className="flex items-center text-sm text-green-600">
                  <span className="animate-pulse mr-2">●</span>
                  实时流式传输
                </span>
              )}
            </div>
            <button
              onClick={handleCloseLogs}
              className="flex items-center px-3 py-1.5 text-sm text-gray-600 hover:text-gray-700 hover:bg-gray-50 rounded transition-colors"
            >
              <XMarkIcon className="w-4 h-4 mr-1" />
              关闭
            </button>
          </div>

          <LogTerminal logs={containerLogs} maxHeight="500px" />

          {containerLogs.length === 0 && !isStreamingLogs && (
            <div className="text-center py-8 text-gray-500">
              暂无日志数据
            </div>
          )}
        </div>
      )}
    </div>
  );
};
