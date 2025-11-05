import React, { useState, useEffect } from 'react';
import { useApp } from '../context/AppContext';
import { LogTerminal } from '../components/LogTerminal';
import { ParameterInputModal } from '../components/ParameterInputModal';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  RocketLaunchIcon,
  Cog6ToothIcon,
  ArrowPathIcon,
} from '@heroicons/react/24/outline';
import { CommandConfig, ParameterInputResult } from '../types';

export const DeployPage: React.FC = () => {
  const { workingDirectory, isDirectoryValid, logs, addLog, isExecuting, setIsExecuting } = useApp();
  const [currentVersion, setCurrentVersion] = useState<string>('未知');
  const [latestVersion, setLatestVersion] = useState<string>('未知');
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [showAdvancedModal, setShowAdvancedModal] = useState(false);
  const [deployLogs, setDeployLogs] = useState(logs);

  // 高级选项配置
  const advancedConfig: CommandConfig = {
    id: 'auto-upgrade-deploy',
    name: '自动升级部署 - 高级选项',
    description: '配置自动升级部署的高级参数',
    parameters: [
      {
        name: 'skip_backup',
        label: '跳过备份',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '跳过升级前的自动备份',
      },
      {
        name: 'force',
        label: '强制升级',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '强制执行升级，即使版本相同',
      },
      {
        name: 'strategy',
        label: '升级策略',
        type: 'select',
        required: false,
        options: [
          { value: 'auto', label: '自动选择' },
          { value: 'full', label: '完整升级' },
          { value: 'patch', label: '增量升级' },
        ],
        defaultValue: 'auto',
        description: '选择升级策略',
      },
    ],
  };

  // 监听 CLI 输出事件
  useEffect(() => {
    const unlistenOutput = listen<string>('cli-output', (event) => {
      addLog({
        type: 'info',
        message: event.payload,
      });
    });

    const unlistenError = listen<string>('cli-error', (event) => {
      addLog({
        type: 'error',
        message: event.payload,
      });
    });

    const unlistenComplete = listen<number>('cli-complete', (event) => {
      const exitCode = event.payload;
      if (exitCode === 0) {
        addLog({
          type: 'success',
          message: '命令执行完成',
        });
      } else {
        addLog({
          type: 'error',
          message: `命令执行失败，退出码: ${exitCode}`,
        });
      }
      setIsExecuting(false);
    });

    return () => {
      unlistenOutput.then((fn) => fn());
      unlistenError.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
    };
  }, [addLog, setIsExecuting]);

  // 同步部署日志
  useEffect(() => {
    setDeployLogs(logs);
  }, [logs]);

  // 检查更新
  const checkUpdate = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsCheckingUpdate(true);

    try {
      addLog({
        type: 'command',
        message: '检查更新...',
        command: 'check-update',
      });

      const result = await invoke<{ success: boolean; stdout: string; stderr: string }>(
        'execute_duck_cli_smart',
        {
          args: ['check-update'],
          workingDir: workingDirectory,
        }
      );

      if (result.success) {
        // 解析版本信息
        const lines = result.stdout.split('\n');
        const currentLine = lines.find((line) => line.includes('当前版本'));
        const latestLine = lines.find((line) => line.includes('最新版本'));

        if (currentLine) {
          const match = currentLine.match(/v?(\d+\.\d+\.\d+(\.\d+)?)/);
          if (match) setCurrentVersion(match[1]);
        }

        if (latestLine) {
          const match = latestLine.match(/v?(\d+\.\d+\.\d+(\.\d+)?)/);
          if (match) setLatestVersion(match[1]);
        }

        addLog({
          type: 'success',
          message: '更新检查完成',
        });
      }
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `检查更新失败: ${errorMessage}`,
      });
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  // 一键部署
  const handleDeploy = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: '开始一键部署...',
        command: 'auto-upgrade-deploy',
        args: ['run'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['auto-upgrade-deploy', 'run'],
        workingDir: workingDirectory,
      });

      // 部署完成后刷新版本信息
      await checkUpdate();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `一键部署失败: ${errorMessage}`,
      });
      setIsExecuting(false);
    }
  };

  // 高级部署
  const handleAdvancedDeploy = (parameters: ParameterInputResult) => {
    setShowAdvancedModal(false);

    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    setIsExecuting(true);

    // 构建命令参数
    const args = ['auto-upgrade-deploy', 'run'];

    if (parameters.skip_backup) {
      args.push('--skip-backup');
    }

    if (parameters.force) {
      args.push('--force');
    }

    if (parameters.strategy && parameters.strategy !== 'auto') {
      args.push('--strategy', parameters.strategy);
    }

    addLog({
      type: 'command',
      message: '开始高级部署...',
      command: 'auto-upgrade-deploy',
      args: args.slice(1),
    });

    invoke('execute_duck_cli_smart', {
      args,
      workingDir: workingDirectory,
    })
      .then(async () => {
        // 部署完成后刷新版本信息
        await checkUpdate();
      })
      .catch((err) => {
        const errorMessage = err instanceof Error ? err.message : String(err);
        addLog({
          type: 'error',
          message: `高级部署失败: ${errorMessage}`,
        });
        setIsExecuting(false);
      });
  };

  // 初始加载时检查更新
  useEffect(() => {
    if (workingDirectory && isDirectoryValid) {
      checkUpdate();
    }
  }, [workingDirectory, isDirectoryValid]);

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
      {/* 版本信息 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold text-gray-900">版本信息</h2>
          <button
            onClick={checkUpdate}
            disabled={isCheckingUpdate || isExecuting}
            className="flex items-center px-3 py-1.5 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <ArrowPathIcon className={`w-4 h-4 mr-1 ${isCheckingUpdate ? 'animate-spin' : ''}`} />
            刷新
          </button>
        </div>

        <div className="grid grid-cols-2 gap-6">
          <div>
            <p className="text-gray-500 text-sm mb-1">当前版本</p>
            <p className="text-2xl font-bold text-gray-900">{currentVersion}</p>
          </div>
          <div>
            <p className="text-gray-500 text-sm mb-1">最新版本</p>
            <p className="text-2xl font-bold text-blue-600">{latestVersion}</p>
          </div>
        </div>

        {currentVersion !== '未知' && latestVersion !== '未知' && currentVersion !== latestVersion && (
          <div className="mt-4 p-3 bg-blue-50 border border-blue-200 rounded-lg">
            <p className="text-blue-700 text-sm">
              🎉 发现新版本！点击"一键部署"升级到最新版本
            </p>
          </div>
        )}
      </div>

      {/* 部署操作 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">部署操作</h2>
        <div className="flex space-x-3">
          <button
            onClick={handleDeploy}
            disabled={isExecuting}
            className="flex items-center px-6 py-3 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RocketLaunchIcon className="w-5 h-5 mr-2" />
            {isExecuting ? '部署中...' : '一键部署'}
          </button>

          <button
            onClick={() => setShowAdvancedModal(true)}
            disabled={isExecuting}
            className="flex items-center px-6 py-3 bg-gray-600 text-white rounded-lg hover:bg-gray-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Cog6ToothIcon className="w-5 h-5 mr-2" />
            高级选项
          </button>
        </div>

        <div className="mt-4 p-3 bg-gray-50 rounded-lg">
          <p className="text-gray-600 text-sm">
            💡 提示：一键部署会自动检测版本、下载更新、备份数据并启动服务
          </p>
        </div>
      </div>

      {/* 部署日志 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">部署日志</h2>
        <LogTerminal logs={deployLogs} maxHeight="500px" />
      </div>

      {/* 高级选项模态框 */}
      <ParameterInputModal
        isOpen={showAdvancedModal}
        commandConfig={advancedConfig}
        onConfirm={handleAdvancedDeploy}
        onCancel={() => setShowAdvancedModal(false)}
      />
    </div>
  );
};
