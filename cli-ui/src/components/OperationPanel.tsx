import React, { useState } from 'react';
import {
  PlayIcon,
  StopIcon,
  ArrowPathIcon,
  ArrowUpTrayIcon,
  WrenchScrewdriverIcon,
  DocumentDuplicateIcon,
  BackwardIcon,
  Cog6ToothIcon
} from '@heroicons/react/24/outline';
import { UpdateManager, DialogManager } from '../utils/tauri';
import ParameterInputModal from './ParameterInputModal';
import { getCommandConfig, needsParameterInput } from '../config/commandConfigs';
import { CommandConfig, ParameterInputResult } from '../types';
import BackupSelectionModal from './BackupSelectionModal';

interface OperationPanelProps {
  workingDirectory: string | null;
  isDirectoryValid: boolean;
  onCommandExecute: (command: string, args: string[]) => Promise<void>;
  onLogMessage: (message: string, type: 'info' | 'success' | 'error' | 'warning') => void;
}

interface ActionButton {
  id: string;
  title: string;
  description: string;
  icon: React.ReactNode;
  action: (parameters?: ParameterInputResult) => Promise<void>;
  variant: 'primary' | 'secondary' | 'success' | 'warning' | 'danger';
  disabled?: boolean;
  commandId?: string; // 对应的命令ID，用于参数输入
}

interface BackupRecord {
  id: number;
  backup_type: 'Manual' | 'PreUpgrade';
  created_at: string;
  service_version: string;
  file_path: string;
  file_size?: number;
  file_exists: boolean;
}

const OperationPanel: React.FC<OperationPanelProps> = ({
  workingDirectory,
  isDirectoryValid,
  onCommandExecute,
  onLogMessage
}) => {
  const [executingActions, setExecutingActions] = useState<Set<string>>(new Set());
  const [parameterModalOpen, setParameterModalOpen] = useState(false);
  const [backupSelectionModalOpen, setBackupSelectionModalOpen] = useState(false);
  const [currentCommand, setCurrentCommand] = useState<{
    actionId: string;
    config: CommandConfig;
    actionFn: (parameters?: ParameterInputResult) => Promise<void>;
  } | null>(null);

  // 检查是否禁用（工作目录无效）
  const isDisabled = !workingDirectory || !isDirectoryValid;

  // 执行操作的包装函数
  const executeAction = async (actionId: string, actionFn: (parameters?: ParameterInputResult) => Promise<void>, commandId?: string) => {
    if (isDisabled) {
      await DialogManager.showMessage('警告', '请先设置有效的工作目录', 'warning');
      return;
    }

    // 检查是否需要参数输入
    if (commandId && needsParameterInput(commandId)) {
      const config = getCommandConfig(commandId);
      if (config) {
        setCurrentCommand({
          actionId,
          config,
          actionFn
        });
        setParameterModalOpen(true);
        return;
      }
    }

    // 直接执行命令（无参数）
    setExecutingActions(prev => new Set(prev).add(actionId));
    
    try {
      await actionFn();
    } catch (error) {
      onLogMessage(`操作失败: ${error}`, 'error');
    } finally {
      setExecutingActions(prev => {
        const newSet = new Set(prev);
        newSet.delete(actionId);
        return newSet;
      });
    }
  };

  // 处理参数输入确认
  const handleParameterConfirm = async (parameters: ParameterInputResult) => {
    if (!currentCommand) return;
    
    setParameterModalOpen(false);
    setExecutingActions(prev => new Set(prev).add(currentCommand.actionId));
    
    try {
      await currentCommand.actionFn(parameters);
    } catch (error) {
      onLogMessage(`操作失败: ${error}`, 'error');
    } finally {
      setExecutingActions(prev => {
        const newSet = new Set(prev);
        newSet.delete(currentCommand.actionId);
        return newSet;
      });
      setCurrentCommand(null);
    }
  };

  // 处理参数输入取消
  const handleParameterCancel = () => {
    setParameterModalOpen(false);
    setCurrentCommand(null);
  };

  // 处理备份选择确认
  const handleBackupSelectionConfirm = async (backupId: number, backupInfo: BackupRecord) => {
    setBackupSelectionModalOpen(false);
    setExecutingActions(prev => new Set(prev).add('rollback'));
    
    try {
      onLogMessage(`🔄 开始回滚到备份 #${backupId}...`, 'info');
      onLogMessage(`📋 备份信息: ${backupInfo.backup_type === 'Manual' ? '手动' : '升级前'}备份, 版本 ${backupInfo.service_version}`, 'info');
      
      // 使用统一的命令执行方式，获得实时输出（就像其他按钮一样）
      const args = ['rollback', backupId.toString(), '--force'];
      await onCommandExecute('duck-cli', args);
      
      onLogMessage(`✅ 回滚操作完成`, 'success');
    } catch (error) {
      onLogMessage(`❌ 回滚操作失败: ${error}`, 'error');
    } finally {
      setExecutingActions(prev => {
        const newSet = new Set(prev);
        newSet.delete('rollback');
        return newSet;
      });
    }
  };

  // 处理备份选择取消
  const handleBackupSelectionCancel = () => {
    setBackupSelectionModalOpen(false);
  };

  // 构建命令行参数
  const buildCommandArgs = (baseArgs: string[], parameters: ParameterInputResult, positionalParams: string[] = []): string[] => {
    const args = [...baseArgs];
    
    // 处理位置参数（如 backup_id, container_name 等）
    positionalParams.forEach(paramName => {
      const value = parameters[paramName];
      if (value !== undefined && value !== null && value !== '') {
        args.push(value.toString());
      }
    });
    
    // 处理选项参数
    for (const [key, value] of Object.entries(parameters)) {
      // 跳过位置参数，它们已经处理过了
      if (positionalParams.includes(key)) continue;
      
      if (value === undefined || value === null || value === '') continue;
      
      if (typeof value === 'boolean') {
        if (value) {
          args.push(`--${key}`);
        }
      } else if (Array.isArray(value)) {
        value.forEach(v => {
          args.push(`--${key}`, v);
        });
      } else {
        // 特殊处理：某些参数名需要转换
        const paramName = key === 'args' ? '' : `--${key}`;
        if (paramName) {
          args.push(paramName, value.toString());
        } else {
          // 对于 args 参数，直接添加值（用于 ducker 命令）
          args.push(value.toString());
        }
      }
    }
    
    return args;
  };

  // 定义所有操作按钮
  const actionButtons: ActionButton[] = [
    // {
    //   id: 'init',
    //   title: '初始化',
    //   description: '初始化 Duck CLI 项目',
    //   icon: <RocketLaunchIcon className="h-5 w-5" />,
    //   variant: 'primary',
    //   commandId: 'init',
    //   action: async (parameters?: ParameterInputResult) => {
    //     onLogMessage('开始初始化项目...', 'info');
        
    //     // 构建命令参数
    //     const baseArgs = ['init'];
    //     const args = parameters ? buildCommandArgs(baseArgs, parameters, []) : baseArgs;
        
    //     // 使用统一的命令执行方式，获得实时输出
    //     await onCommandExecute('duck-cli', args);
    //   }
    // },
    // {
    //   id: 'download',
    //   title: '下载Docker应用',
    //   description: '下载 Docker 应用文件,支持全量下载和强制重新下载',
    //   icon: <CloudArrowDownIcon className="h-5 w-5" />,
    //   variant: 'secondary',
    //   commandId: 'upgrade',
    //   action: async (parameters?: ParameterInputResult) => {
    //     onLogMessage('📥 准备下载Docker服务...', 'info');
        
    //     // 默认使用全量下载，除非用户指定了其他参数
    //     const defaultParams = { full: true, ...parameters };
    //     const baseArgs = ['upgrade'];
    //     const args = buildCommandArgs(baseArgs, defaultParams, []);
        
    //     // 只需要调用onCommandExecute，它现在会真正执行命令并显示实时输出
    //     await onCommandExecute('duck-cli', args);
    //   }
    // },
    {
      id: 'deploy',
      title: '一键部署',
      description: '智能一键部署：自动执行初始化 + 自动升级部署流程',
      icon: <ArrowUpTrayIcon className="h-5 w-5" />,
      variant: 'primary',
      commandId: 'auto-upgrade-deploy',
      action: async (parameters?: ParameterInputResult) => {
        // 检查工作目录是否设置
        if (!workingDirectory) {
          onLogMessage('❌ 工作目录未设置，无法执行部署', 'error');
          return;
        }

        onLogMessage('🚀 智能一键部署开始...', 'info');
        onLogMessage('📋 步骤 1/2: 执行项目初始化（duck-cli 会自动检查是否已初始化）', 'info');
        
        // 始终先执行初始化，duck-cli init 内部有防重复逻辑
        try {
          await onCommandExecute('duck-cli', ['init']);
          onLogMessage('✅ 初始化步骤完成！', 'success');
        } catch (error) {
          onLogMessage(`❌ 初始化失败，一键部署终止: ${error}`, 'error');
          return;
        }
        
        onLogMessage('📋 步骤 2/2: 执行自动升级部署...', 'info');
        
        // 构建命令参数
        const baseArgs = ['auto-upgrade-deploy', 'run'];
        const args = parameters ? buildCommandArgs(baseArgs, parameters, []) : baseArgs;
        
        // 使用统一的命令执行方式，获得实时输出
        await onCommandExecute('duck-cli', args);
      }
    },
    {
      id: 'start',
      title: '启动服务',
      description: '启动 Docker 服务',
      icon: <PlayIcon className="h-5 w-5" />,
      variant: 'success',
      action: async () => {
        onLogMessage('🚀 启动服务...', 'info');
        await onCommandExecute('duck-cli', ['docker-service', 'start']);
      }
    },
    {
      id: 'stop',
      title: '停止服务',
      description: '停止 Docker 服务',
      icon: <StopIcon className="h-5 w-5" />,
      variant: 'warning',
      action: async () => {
        onLogMessage('⏹️ 停止服务...', 'info');
        await onCommandExecute('duck-cli', ['docker-service', 'stop']);
      }
    },
    {
      id: 'restart',
      title: '重启服务',
      description: '重启 Docker 服务',
      icon: <ArrowPathIcon className="h-5 w-5" />,
      variant: 'secondary',
      action: async () => {
        onLogMessage('🔄 重启服务...', 'info');
        await onCommandExecute('duck-cli', ['docker-service', 'restart']);
      }
    },

    {
      id: 'auto-backup',
      title: '数据备份',
      description: '自动备份Docker服务使用的数据（会先停止服务，备份完成后自动启动）',
      icon: <DocumentDuplicateIcon className="h-5 w-5" />,
      variant: 'secondary',
      commandId: 'auto-backup',
      action: async (parameters?: ParameterInputResult) => {
        // 确认操作
        const confirmed = await DialogManager.confirmAction(
          '创建服务备份',
          '此操作将会停止Docker服务进行备份，备份完成后会自动重启服务。\n\n确定要继续吗？'
        );
        
        if (!confirmed) {
          return;
        }
        
        onLogMessage('💾 开始自动备份流程...', 'info');
        onLogMessage('⚠️ 提醒：备份过程中服务将暂时停止', 'warning');
        
        // 构建命令参数
        const baseArgs = ['auto-backup', 'run'];
        const args = parameters ? buildCommandArgs(baseArgs, parameters, []) : baseArgs;
        
        await onCommandExecute('duck-cli', args);
      }
    },
    {
      id: 'rollback',
      title: '数据回滚',
      description: '选择备份版本并回滚Docker服务使用的数据',
      icon: <BackwardIcon className="h-5 w-5" />,
      variant: 'warning',
      action: async () => {
        // 打开备份选择模态框
        setBackupSelectionModalOpen(true);
      }
    },

    {
      id: 'upgrade',
      title: '应用升级',
      description: '下载Docker应用服务文件，支持全量下载和强制重新下载',
      icon: <WrenchScrewdriverIcon className="h-5 w-5" />,
      variant: 'primary',
      commandId: 'upgrade',
      action: async (parameters?: ParameterInputResult) => {
        onLogMessage('🔧 升级服务...', 'info');
        
        // 构建命令参数
        const baseArgs = ['upgrade'];
        const args = parameters ? buildCommandArgs(baseArgs, parameters, []) : baseArgs;
        
        // 使用统一的命令执行方式，获得实时输出
        await onCommandExecute('duck-cli', args);
      }
    },
    {
      id: 'app-update',
      title: '客户端更新',
      description: '检查并更新客户端',
      icon: <Cog6ToothIcon className="h-5 w-5" />,
      variant: 'primary',
      action: async () => {
        onLogMessage('检查客户端更新...', 'info');
        
        try {
          const update = await UpdateManager.checkForUpdates();
          if (update) {
            const confirmed = await DialogManager.confirmAction(
              '发现新版本',
              `发现新版本 ${update.version}，是否立即更新？`
            );
            
            if (confirmed) {
              onLogMessage('下载并安装更新...', 'info');
              await UpdateManager.downloadAndInstallUpdate((downloaded, total) => {
                if (total > 0) {
                  const progress = ((downloaded / total) * 100).toFixed(1);
                  onLogMessage(`下载进度: ${progress}% (${downloaded}/${total} bytes)`, 'info');
                } else {
                  onLogMessage(`下载中: ${downloaded} bytes`, 'info');
                }
              });
              onLogMessage('更新完成，应用即将重启', 'success');
            }
          } else {
            onLogMessage('已是最新版本', 'info');
          }
        } catch (error) {
          onLogMessage(`更新检查失败: ${error}`, 'error');
        }
      }
    }
  ];

  // 获取按钮样式
  const getButtonStyle = (variant: string, disabled: boolean, executing: boolean) => {
    const baseClasses = "relative inline-flex items-center px-3 py-2 border text-sm font-medium rounded-lg focus:outline-none focus:ring-2 focus:ring-offset-2 transition-all duration-200 h-20 w-full";
    
    if (disabled) {
      return `${baseClasses} border-gray-200 text-gray-400 bg-gray-50 cursor-not-allowed`;
    }
    
    if (executing) {
      return `${baseClasses} border-blue-300 text-blue-700 bg-blue-50 cursor-wait`;
    }

    switch (variant) {
      case 'primary':
        return `${baseClasses} border-blue-300 text-blue-700 bg-blue-50 hover:bg-blue-100 focus:ring-blue-500`;
      case 'success':
        return `${baseClasses} border-green-300 text-green-700 bg-green-50 hover:bg-green-100 focus:ring-green-500`;
      case 'warning':
        return `${baseClasses} border-yellow-300 text-yellow-700 bg-yellow-50 hover:bg-yellow-100 focus:ring-yellow-500`;
      case 'danger':
        return `${baseClasses} border-red-300 text-red-700 bg-red-50 hover:bg-red-100 focus:ring-red-500`;
      default:
        return `${baseClasses} border-gray-300 text-gray-700 bg-gray-50 hover:bg-gray-100 focus:ring-gray-500`;
    }
  };

  return (
    <div className="bg-white p-4 sm:p-6">
      <div className="mb-3 sm:mb-4">
        <h2 className="text-lg font-semibold text-gray-900">操作面板</h2>
        <p className="text-sm text-gray-600 mt-1">
          {isDisabled ? '请先设置有效的工作目录' : '选择要执行的操作'}
        </p>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-3 sm:gap-4 md:gap-5 auto-rows-fr">
        {actionButtons.map((button) => {
          const isExecuting = executingActions.has(button.id);
          const isButtonDisabled = isDisabled || isExecuting;

          return (
            <button
              key={button.id}
              onClick={() => executeAction(button.id, button.action, button.commandId)}
              disabled={isButtonDisabled}
              className={getButtonStyle(button.variant, isButtonDisabled, isExecuting)}
              title={button.description}
            >
              <div className="flex flex-col items-center text-center w-full">
                <div className="mb-2 relative">
                  {isExecuting ? (
                    <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-current"></div>
                  ) : (
                    button.icon
                  )}
                </div>
                <span className="text-xs font-medium">{button.title}</span>
              </div>
            </button>
          );
        })}
      </div>

      {/* 状态提示 */}
      {isDisabled && (
        <div className="mt-4 p-3 bg-yellow-50 border border-yellow-200 rounded-md">
          <div className="flex">
            <div className="flex-shrink-0">
              <svg className="h-5 w-5 text-yellow-400" viewBox="0 0 20 20" fill="currentColor">
                <path fillRule="evenodd" d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
              </svg>
            </div>
            <div className="ml-3">
              <p className="text-sm text-yellow-800">
                工作目录未设置或无效，所有操作已禁用。请在顶部选择有效的工作目录。
              </p>
            </div>
          </div>
        </div>
      )}

      {/* 参数输入模态框 */}
      <ParameterInputModal
        isOpen={parameterModalOpen}
        commandConfig={currentCommand?.config || null}
        onConfirm={handleParameterConfirm}
        onCancel={handleParameterCancel}
      />

      {/* 备份选择模态框 */}
      <BackupSelectionModal
        isOpen={backupSelectionModalOpen}
        workingDirectory={workingDirectory || ''}
        onConfirm={handleBackupSelectionConfirm}
        onCancel={handleBackupSelectionCancel}
      />
    </div>
  );
};

export default OperationPanel; 