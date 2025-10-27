import React, { useState, useEffect } from 'react';
import { XMarkIcon, DocumentDuplicateIcon, ClockIcon, TagIcon } from '@heroicons/react/24/outline';
import { DuckCliManager } from '../utils/tauri';

interface BackupRecord {
  id: number;
  backup_type: 'Manual' | 'PreUpgrade';
  created_at: string;
  service_version: string;
  file_path: string;
  file_size?: number;
  file_exists: boolean;
}

interface BackupSelectionModalProps {
  isOpen: boolean;
  workingDirectory: string;
  onConfirm: (backupId: number, backupInfo: BackupRecord) => void;
  onCancel: () => void;
}

const BackupSelectionModal: React.FC<BackupSelectionModalProps> = ({
  isOpen,
  workingDirectory,
  onConfirm,
  onCancel
}) => {
  const [backups, setBackups] = useState<BackupRecord[]>([]);
  const [selectedBackup, setSelectedBackup] = useState<BackupRecord | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>('');

  // 获取备份列表
  const fetchBackups = async () => {
    if (!workingDirectory) return;
    
    setLoading(true);
    setError('');
    
    try {
      console.log('使用新的 Tauri 命令获取备份列表...');
      const result = await DuckCliManager.getBackupList(workingDirectory);
      
      console.log('备份列表结果:', result);
      
      if (result.success) {
        // 直接使用返回的结构化数据
        const backupList = result.backups.map(backup => ({
          id: backup.id,
          backup_type: backup.backup_type as 'Manual' | 'PreUpgrade',
          created_at: backup.created_at,
          service_version: backup.service_version,
          file_path: backup.file_path,
          file_size: backup.file_size,
          file_exists: backup.file_exists
        }));
        
        setBackups(backupList);
        
        if (backupList.length === 0) {
          setError('没有可用的备份');
        }
      } else {
        setError(result.error || '获取备份列表失败');
      }
    } catch (err) {
      console.error('获取备份失败:', err);
      setError(`获取备份失败: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  // 格式化文件大小显示
  const formatFileSize = (bytes?: number): string => {
    if (!bytes) return '未知';
    
    if (bytes >= 1024 * 1024 * 1024) {
      return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
    } else if (bytes >= 1024 * 1024) {
      return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    } else if (bytes >= 1024) {
      return `${(bytes / 1024).toFixed(1)} KB`;
    } else {
      return `${bytes} B`;
    }
  };

  // 格式化备份类型
  const formatBackupType = (type: string): string => {
    return type === 'Manual' ? '手动备份' : '升级前备份';
  };

  // 获取备份类型颜色
  const getBackupTypeColor = (type: string): string => {
    return type === 'Manual' 
      ? 'bg-blue-100 text-blue-800' 
      : 'bg-purple-100 text-purple-800';
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
        second: '2-digit'
      });
    } catch {
      return dateTime;
    }
  };

  // 处理确认
  const handleConfirm = () => {
    if (selectedBackup) {
      onConfirm(selectedBackup.id, selectedBackup);
    }
  };

  // 组件挂载时获取备份列表
  useEffect(() => {
    if (isOpen && workingDirectory) {
      fetchBackups();
    }
  }, [isOpen, workingDirectory]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto">
      <div className="flex items-center justify-center min-h-screen pt-4 px-4 pb-20 text-center sm:block sm:p-0">
        {/* 背景遮罩 */}
        <div className="fixed inset-0 bg-gray-500 bg-opacity-75 transition-opacity" onClick={onCancel} />

        {/* 模态框 */}
        <div className="inline-block align-bottom bg-white rounded-lg text-left overflow-hidden shadow-xl transform transition-all sm:my-8 sm:align-middle sm:max-w-4xl sm:w-full">
          
          {/* 标题栏 */}
          <div className="flex items-center justify-between px-6 py-4 border-b border-gray-200">
            <div className="flex items-center space-x-3">
              <DocumentDuplicateIcon className="h-6 w-6 text-blue-500" />
              <h3 className="text-lg font-medium text-gray-900">
                选择要恢复的备份
              </h3>
            </div>
            <button
              onClick={onCancel}
              className="text-gray-400 hover:text-gray-500"
            >
              <XMarkIcon className="h-6 w-6" />
            </button>
          </div>

          {/* 内容区域 */}
          <div className="px-6 py-4" style={{ maxHeight: '60vh', overflowY: 'auto' }}>
            {loading && (
              <div className="flex items-center justify-center py-8">
                <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
                <span className="ml-3 text-gray-600">正在获取备份列表...</span>
              </div>
            )}

            {error && (
              <div className="bg-red-50 border border-red-200 rounded-md p-4 mb-4">
                <div className="flex">
                  <div className="ml-3">
                    <h3 className="text-sm font-medium text-red-800">
                      获取备份列表失败
                    </h3>
                    <div className="mt-2 text-sm text-red-700">
                      {error}
                    </div>
                  </div>
                </div>
              </div>
            )}

            {!loading && !error && backups.length === 0 && (
              <div className="text-center py-8">
                <DocumentDuplicateIcon className="mx-auto h-12 w-12 text-gray-400" />
                <h3 className="mt-2 text-sm font-medium text-gray-900">没有可用的备份</h3>
                <p className="mt-1 text-sm text-gray-500">
                  请先创建备份，然后再尝试恢复操作
                </p>
              </div>
            )}

            {!loading && !error && backups.length > 0 && (
              <div className="space-y-3">
                <p className="text-sm text-gray-600 mb-4">
                  找到 {backups.filter(b => b.file_exists).length} 个可用备份，请选择要恢复的版本：
                </p>
                
                {backups.map((backup) => (
                  <div
                    key={backup.id}
                    className={`relative rounded-lg border-2 p-4 cursor-pointer transition-all ${
                      selectedBackup?.id === backup.id
                        ? 'border-blue-500 bg-blue-50'
                        : backup.file_exists
                        ? 'border-gray-200 hover:border-gray-300 hover:bg-gray-50'
                        : 'border-gray-200 bg-gray-100 cursor-not-allowed opacity-60'
                    }`}
                    onClick={() => backup.file_exists && setSelectedBackup(backup)}
                  >
                    <div className="flex items-start justify-between">
                      <div className="flex-1">
                        <div className="flex items-center space-x-3">
                          <div className="flex items-center space-x-2">
                            <span className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${getBackupTypeColor(backup.backup_type)}`}>
                              {formatBackupType(backup.backup_type)}
                            </span>
                            <span className="text-sm font-medium text-gray-900">
                              备份 #{backup.id}
                            </span>
                          </div>
                          {!backup.file_exists && (
                            <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800">
                              文件缺失
                            </span>
                          )}
                        </div>
                        
                        <div className="mt-2 grid grid-cols-1 md:grid-cols-3 gap-4 text-sm text-gray-600">
                          <div className="flex items-center space-x-2">
                            <ClockIcon className="h-4 w-4" />
                            <span>{formatDateTime(backup.created_at)}</span>
                          </div>
                          <div className="flex items-center space-x-2">
                            <TagIcon className="h-4 w-4" />
                            <span>版本 {backup.service_version}</span>
                          </div>
                          <div className="flex items-center space-x-2">
                            <DocumentDuplicateIcon className="h-4 w-4" />
                            <span>{formatFileSize(backup.file_size)}</span>
                          </div>
                        </div>
                        
                        <div className="mt-2 text-xs text-gray-500 truncate">
                          文件: {backup.file_path}
                        </div>
                      </div>
                      
                      {selectedBackup?.id === backup.id && (
                        <div className="flex-shrink-0 ml-4">
                          <div className="h-5 w-5 rounded-full bg-blue-500 flex items-center justify-center">
                            <div className="h-2 w-2 rounded-full bg-white"></div>
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* 选中备份的详细信息 */}
          {selectedBackup && (
            <div className="px-6 py-4 bg-blue-50 border-t border-blue-200">
              <h4 className="text-sm font-medium text-blue-900 mb-2">将要恢复的备份信息：</h4>
              <div className="text-sm text-blue-800 space-y-1">
                <div>备份ID: {selectedBackup.id}</div>
                <div>类型: {formatBackupType(selectedBackup.backup_type)}</div>
                <div>创建时间: {formatDateTime(selectedBackup.created_at)}</div>
                <div>服务版本: {selectedBackup.service_version}</div>
                <div>文件大小: {formatFileSize(selectedBackup.file_size)}</div>
              </div>
            </div>
          )}

          {/* 按钮区域 */}
          <div className="px-6 py-4 bg-gray-50 border-t border-gray-200 flex justify-end space-x-3">
            <button
              onClick={onCancel}
              className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
            >
              取消
            </button>
            <button
              onClick={handleConfirm}
              disabled={!selectedBackup || !selectedBackup.file_exists}
              className={`px-4 py-2 text-sm font-medium text-white rounded-md focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 ${
                selectedBackup && selectedBackup.file_exists
                  ? 'bg-blue-600 hover:bg-blue-700'
                  : 'bg-gray-400 cursor-not-allowed'
              }`}
            >
              确认恢复
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default BackupSelectionModal; 