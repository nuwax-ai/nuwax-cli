import React from 'react';
import { FolderIcon, CheckCircleIcon, XCircleIcon, ExclamationTriangleIcon } from '@heroicons/react/24/outline';
import { ValidationState } from '../store/appStore';

interface WorkingDirectoryBarProps {
  workingDirectory: string | null;
  validationState: ValidationState;
  validationError?: string;
  onSelectDirectory: () => Promise<void>;
  onRecheck: () => Promise<void>;
  isRechecking: boolean;
}

const WorkingDirectoryBar: React.FC<WorkingDirectoryBarProps> = ({ 
  workingDirectory,
  validationState,
  validationError,
  onSelectDirectory,
  onRecheck,
  isRechecking
}) => {

  // 获取状态图标
  const getStatusIcon = () => {
    switch (validationState) {
      case 'valid':
        return <CheckCircleIcon className="h-5 w-5 text-green-500" />;
      case 'invalid':
        return <XCircleIcon className="h-5 w-5 text-red-500" />;
      case 'validating':
        return (
          <div className="animate-spin rounded-full h-5 w-5 border-b-2 border-blue-500"></div>
        );
      default:
        return <ExclamationTriangleIcon className="h-5 w-5 text-yellow-500" />;
    }
  };

  // 获取状态文本
  const getStatusText = () => {
    switch (validationState) {
      case 'valid':
        return '目录有效';
      case 'invalid':
        return `无效: ${validationError}`;
      case 'validating':
        return '验证中...';
      default:
        return '未设置工作目录';
    }
  };

  // 获取状态颜色
  const getStatusColor = () => {
    switch (validationState) {
      case 'valid':
        return 'text-green-600 bg-green-50';
      case 'invalid':
        return 'text-red-600 bg-red-50';
      case 'validating':
        return 'text-blue-600 bg-blue-50';
      default:
        return 'text-yellow-600 bg-yellow-50';
    }
  };

  return (
    <div className="bg-white border-b border-gray-200 px-4 py-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center space-x-3 flex-1 min-w-0">
          <FolderIcon className="h-5 w-5 text-gray-500 flex-shrink-0" />
          
          {/* 工作目录路径 */}
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium text-gray-900 truncate">
              {workingDirectory || '未选择工作目录'}
            </div>
            <div className={`text-xs px-2 py-1 rounded-full inline-flex items-center space-x-1 mt-1 ${getStatusColor()}`}>
              {getStatusIcon()}
              <span>{getStatusText()}</span>
            </div>
          </div>
        </div>

        {/* 选择目录按钮 */}
        <button
          onClick={onSelectDirectory}
          className="ml-3 inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
        >
          <FolderIcon className="h-4 w-4 mr-2" />
          选择目录
        </button>
        <button
          onClick={onRecheck}
          disabled={!workingDirectory || isRechecking}
          className="ml-3 inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {isRechecking ? '检测中...' : '重新检测'}
        </button>
      </div>

      {/* 详细错误信息 */}
      {validationState === 'invalid' && validationError && (
        <div className="mt-2 p-2 bg-red-50 border border-red-200 rounded-md">
          <div className="flex">
            <XCircleIcon className="h-5 w-5 text-red-400 flex-shrink-0" />
            <div className="ml-2">
              <h3 className="text-sm font-medium text-red-800">目录验证失败</h3>
              <p className="text-sm text-red-700 mt-1">{validationError}</p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default WorkingDirectoryBar; 