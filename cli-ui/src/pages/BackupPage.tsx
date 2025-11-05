import React, { useState, useEffect } from 'react';
import { useApp } from '../context/AppContext';
import BackupSelectionModal from '../components/BackupSelectionModal';
import { invoke } from '@tauri-apps/api/core';
import {
  PlusIcon,
  ArrowPathIcon,
  TrashIcon,
  ClockIcon,
  TagIcon,
  DocumentDuplicateIcon,
} from '@heroicons/react/24/outline';

interface BackupRecordInfo {
  id: number;
  backup_type: string;
  created_at: string;
  service_version: string;
  file_path: string;
  status: string;
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

export const BackupPage: React.FC = () => {
  const { workingDirectory, isDirectoryValid, addLog, isExecuting, setIsExecuting } = useApp();
  const [backups, setBackups] = useState<BackupRecordInfo[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showRestoreModal, setShowRestoreModal] = useState(false);

  // 获取备份列表
  const fetchBackups = async () => {
    if (!workingDirectory || !isDirectoryValid) {
      return;
    }

    setIsLoading(true);
    setError(null);

    try {
      const result = await invoke<BackupRecordInfo[]>('list_backups', {
        workingDirectory,
      });

      setBackups(result);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(errorMessage);
      console.error('获取备份列表失败:', err);
    } finally {
      setIsLoading(false);
    }
  };

  // 创建备份
  const handleCreateBackup = async () => {
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
        message: '创建备份...',
        command: 'backup',
        args: ['create'],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['backup', 'create'],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '备份创建成功',
      });

      // 刷新备份列表
      await fetchBackups();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `创建备份失败: ${errorMessage}`,
      });
    } finally {
      setIsExecuting(false);
    }
  };

  // 恢复备份
  const handleRestoreBackup = async (backupId: number, backupInfo: BackupRecord) => {
    setShowRestoreModal(false);

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
        message: `恢复备份 #${backupId} (版本: ${backupInfo.service_version})...`,
        command: 'backup',
        args: ['restore', backupId.toString()],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['backup', 'restore', backupId.toString()],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '备份恢复成功',
      });

      // 刷新备份列表
      await fetchBackups();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `恢复备份失败: ${errorMessage}`,
      });
    } finally {
      setIsExecuting(false);
    }
  };

  // 删除备份
  const handleDeleteBackup = async (backupId: number) => {
    if (!workingDirectory || !isDirectoryValid) {
      addLog({
        type: 'error',
        message: '请先设置有效的工作目录',
      });
      return;
    }

    // 确认删除
    const confirmed = window.confirm(`确定要删除备份 #${backupId} 吗？此操作不可恢复。`);
    if (!confirmed) {
      return;
    }

    setIsExecuting(true);

    try {
      addLog({
        type: 'command',
        message: `删除备份 #${backupId}...`,
        command: 'backup',
        args: ['delete', backupId.toString()],
      });

      await invoke('execute_duck_cli_smart', {
        args: ['backup', 'delete', backupId.toString()],
        workingDir: workingDirectory,
      });

      addLog({
        type: 'success',
        message: '备份删除成功',
      });

      // 刷新备份列表
      await fetchBackups();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      addLog({
        type: 'error',
        message: `删除备份失败: ${errorMessage}`,
      });
    } finally {
      setIsExecuting(false);
    }
  };

  // 格式化时间显示
  const formatDateTime = (dateTime: string): string => {
    try {
      const date = new Date(dateTime);
      return date.toLocaleString('zh-CN', {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      });
    } catch {
      return dateTime;
    }
  };

  // 获取备份类型颜色
  const getBackupTypeColor = (type: string): string => {
    return type === '手动备份'
      ? 'bg-blue-100 text-blue-800'
      : 'bg-purple-100 text-purple-800';
  };

  // 获取状态颜色
  const getStatusColor = (status: string): string => {
    return status === '已完成'
      ? 'bg-green-100 text-green-800'
      : 'bg-red-100 text-red-800';
  };

  // 初始加载备份列表
  useEffect(() => {
    if (workingDirectory && isDirectoryValid) {
      fetchBackups();
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
      {/* 操作栏 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-xl font-semibold text-gray-900 mb-2">备份管理</h2>
            <p className="text-gray-500 text-sm">
              创建和管理服务数据备份，支持一键恢复
            </p>
          </div>
          <div className="flex space-x-3">
            <button
              onClick={fetchBackups}
              disabled={isLoading || isExecuting}
              className="flex items-center px-4 py-2 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <ArrowPathIcon className={`w-4 h-4 mr-1 ${isLoading ? 'animate-spin' : ''}`} />
              刷新
            </button>
            <button
              onClick={handleCreateBackup}
              disabled={isExecuting}
              className="flex items-center px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <PlusIcon className="w-5 h-5 mr-2" />
              创建备份
            </button>
          </div>
        </div>
      </div>

      {/* 备份列表 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">备份列表</h2>

        {error && (
          <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-lg">
            <p className="text-red-600 text-sm">{error}</p>
          </div>
        )}

        {isLoading && backups.length === 0 ? (
          <div className="text-center py-8">
            <div className="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
            <p className="text-gray-500 mt-2">加载备份列表...</p>
          </div>
        ) : backups.length === 0 ? (
          <div className="text-center py-8">
            <DocumentDuplicateIcon className="mx-auto h-12 w-12 text-gray-400" />
            <p className="text-gray-500 mt-2">暂无备份</p>
            <p className="text-gray-400 text-sm mt-1">
              点击"创建备份"按钮创建第一个备份
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            {backups.map((backup) => (
              <div
                key={backup.id}
                className="border border-gray-200 rounded-lg p-4 hover:shadow-md transition-shadow"
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center space-x-3 mb-2">
                      <span className="text-lg font-semibold text-gray-900">
                        备份 #{backup.id}
                      </span>
                      <span
                        className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${getBackupTypeColor(
                          backup.backup_type
                        )}`}
                      >
                        {backup.backup_type}
                      </span>
                      <span
                        className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${getStatusColor(
                          backup.status
                        )}`}
                      >
                        {backup.status}
                      </span>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-sm text-gray-600">
                      <div className="flex items-center space-x-2">
                        <ClockIcon className="h-4 w-4" />
                        <span>{formatDateTime(backup.created_at)}</span>
                      </div>
                      <div className="flex items-center space-x-2">
                        <TagIcon className="h-4 w-4" />
                        <span>版本 {backup.service_version}</span>
                      </div>
                    </div>

                    <div className="mt-2 text-xs text-gray-500 truncate">
                      文件: {backup.file_path}
                    </div>
                  </div>

                  <div className="flex items-center space-x-2 ml-4">
                    <button
                      onClick={() => setShowRestoreModal(true)}
                      disabled={isExecuting || backup.status !== '已完成'}
                      className="flex items-center px-3 py-1.5 text-sm text-green-600 hover:text-green-700 hover:bg-green-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      <ArrowPathIcon className="w-4 h-4 mr-1" />
                      恢复
                    </button>
                    <button
                      onClick={() => handleDeleteBackup(backup.id)}
                      disabled={isExecuting}
                      className="flex items-center px-3 py-1.5 text-sm text-red-600 hover:text-red-700 hover:bg-red-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      <TrashIcon className="w-4 h-4 mr-1" />
                      删除
                    </button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* 备份恢复模态框 */}
      {workingDirectory && (
        <BackupSelectionModal
          isOpen={showRestoreModal}
          workingDirectory={workingDirectory}
          onConfirm={handleRestoreBackup}
          onCancel={() => setShowRestoreModal(false)}
        />
      )}
    </div>
  );
};
