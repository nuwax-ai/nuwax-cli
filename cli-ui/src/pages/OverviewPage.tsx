import React, { useEffect, useState } from 'react';
import { useApp } from '../context/AppContext';
import { useContainers } from '../hooks/useContainers';
import { ContainerCard } from '../components/ContainerCard';
import { invoke } from '@tauri-apps/api/core';
import {
  RocketLaunchIcon,
  PlayIcon,
  StopIcon,
  ArrowPathIcon,
} from '@heroicons/react/24/outline';

export const OverviewPage: React.FC = () => {
  const { workingDirectory, isDirectoryValid, addLog, setIsExecuting } = useApp();
  const { containers, isLoading, error, refreshContainers } = useContainers(true, 5000);
  const [isOperating, setIsOperating] = useState(false);

  // 计算服务整体状态
  const runningCount = containers.filter(c => c.status === 'running').length;
  const totalCount = containers.length;
  const allRunning = totalCount > 0 && runningCount === totalCount;
  const allStopped = runningCount === 0;

  const serviceStatus = allRunning
    ? '运行中'
    : allStopped
    ? '已停止'
    : '部分运行';

  const statusColor = allRunning
    ? 'text-green-600'
    : allStopped
    ? 'text-gray-600'
    : 'text-yellow-600';

  const statusIcon = allRunning ? '🟢' : allStopped ? '⚫' : '🟡';

  // 一键部署
  const handleDeploy = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsOperating(true);
    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: '执行一键部署...',
        command: 'auto-upgrade-deploy',
        args: ['run'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['auto-upgrade-deploy', 'run'],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '一键部署完成',
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `一键部署失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
      setIsExecuting(false);
    }
  };

  // 启动所有服务
  const handleStartAll = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsOperating(true);
    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: '启动所有服务...',
        command: 'docker-service',
        args: ['start'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['docker-service', 'start'],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '所有服务启动成功',
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `启动服务失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
      setIsExecuting(false);
    }
  };

  // 停止所有服务
  const handleStopAll = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsOperating(true);
    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: '停止所有服务...',
        command: 'docker-service',
        args: ['stop'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['docker-service', 'stop'],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '所有服务停止成功',
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `停止服务失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
      setIsExecuting(false);
    }
  };

  // 重启所有服务
  const handleRestartAll = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsOperating(true);
    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: '重启所有服务...',
        command: 'docker-service',
        args: ['restart'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['docker-service', 'restart'],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '所有服务重启成功',
      });

      // 刷新容器状态
      await refreshContainers();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `重启服务失败: ${errorMessage}`,
      });
    } finally {
      setIsOperating(false);
      setIsExecuting(false);
    }
  };

  // 容器操作处理
  const handleViewLogs = (containerName: string) => {
    addLog({
      type: 'info',
      message: `查看容器日志: ${containerName}`,
    });
    // TODO: 导航到容器页面并显示该容器的日志
  };

  const handleStartContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('start_container', {
        app: window.__TAURI__?.app,
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

  const handleStopContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('stop_container', {
        app: window.__TAURI__?.app,
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

  const handleRestartContainer = async (containerName: string) => {
    setIsOperating(true);

    try {
      await invoke('restart_container', {
        app: window.__TAURI__?.app,
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
      {/* 服务整体状态 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">服务状态</h2>
        <div className="flex items-center space-x-4">
          <span className="text-3xl">{statusIcon}</span>
          <div>
            <p className={`text-2xl font-bold ${statusColor}`}>{serviceStatus}</p>
            <p className="text-gray-500 text-sm">
              {runningCount} / {totalCount} 个容器运行中
            </p>
          </div>
        </div>
      </div>

      {/* 快速操作 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">快速操作</h2>
        <div className="flex flex-wrap gap-3">
          <button
            onClick={handleDeploy}
            disabled={isOperating || isLoading}
            className="flex items-center px-6 py-3 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RocketLaunchIcon className="w-5 h-5 mr-2" />
            一键部署
          </button>

          <button
            onClick={handleStartAll}
            disabled={isOperating || isLoading || allRunning}
            className="flex items-center px-6 py-3 bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <PlayIcon className="w-5 h-5 mr-2" />
            启动服务
          </button>

          <button
            onClick={handleStopAll}
            disabled={isOperating || isLoading || allStopped}
            className="flex items-center px-6 py-3 bg-red-600 text-white rounded-lg hover:bg-red-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <StopIcon className="w-5 h-5 mr-2" />
            停止服务
          </button>

          <button
            onClick={handleRestartAll}
            disabled={isOperating || isLoading || allStopped}
            className="flex items-center px-6 py-3 bg-orange-600 text-white rounded-lg hover:bg-orange-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <ArrowPathIcon className="w-5 h-5 mr-2" />
            重启服务
          </button>
        </div>
      </div>

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
              <ContainerCard
                key={container.name}
                container={container}
                onViewLogs={handleViewLogs}
                onStart={handleStartContainer}
                onStop={handleStopContainer}
                onRestart={handleRestartContainer}
                isLoading={isOperating}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
