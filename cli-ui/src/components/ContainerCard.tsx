import React from 'react';
import { ContainerInfo } from '../types';
import { StatusBadge } from './StatusBadge';
import {
  PlayIcon,
  StopIcon,
  ArrowPathIcon,
  DocumentTextIcon,
} from '@heroicons/react/24/outline';

interface ContainerCardProps {
  container: ContainerInfo;
  onViewLogs?: (containerName: string) => void;
  onStart?: (containerName: string) => void;
  onStop?: (containerName: string) => void;
  onRestart?: (containerName: string) => void;
  isLoading?: boolean;
}

export const ContainerCard: React.FC<ContainerCardProps> = ({
  container,
  onViewLogs,
  onStart,
  onStop,
  onRestart,
  isLoading = false,
}) => {
  const canStart = container.status === 'stopped' || container.status === 'exited';
  const canStop = container.status === 'running';
  const canRestart = container.status === 'running';

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-4 hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between mb-3">
        <div className="flex-1">
          <h3 className="text-lg font-semibold text-gray-900 mb-1">{container.name}</h3>
          <StatusBadge status={container.status} />
        </div>
      </div>

      <div className="space-y-2 mb-4">
        <div className="flex items-center text-sm">
          <span className="text-gray-500 w-16">镜像:</span>
          <span className="text-gray-900 font-mono text-xs">{container.image}</span>
        </div>
        
        {container.ports && container.ports.length > 0 && (
          <div className="flex items-center text-sm">
            <span className="text-gray-500 w-16">端口:</span>
            <span className="text-gray-900 font-mono text-xs">
              {container.ports.join(', ')}
            </span>
          </div>
        )}

        {container.uptime && (
          <div className="flex items-center text-sm">
            <span className="text-gray-500 w-16">运行时间:</span>
            <span className="text-gray-900 text-xs">{container.uptime}</span>
          </div>
        )}
      </div>

      <div className="flex items-center space-x-2">
        {onViewLogs && (
          <button
            onClick={() => onViewLogs(container.name)}
            disabled={isLoading}
            className="flex items-center px-3 py-1.5 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <DocumentTextIcon className="w-4 h-4 mr-1" />
            查看日志
          </button>
        )}

        {onStart && canStart && (
          <button
            onClick={() => onStart(container.name)}
            disabled={isLoading}
            className="flex items-center px-3 py-1.5 text-sm text-green-600 hover:text-green-700 hover:bg-green-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <PlayIcon className="w-4 h-4 mr-1" />
            启动
          </button>
        )}

        {onStop && canStop && (
          <button
            onClick={() => onStop(container.name)}
            disabled={isLoading}
            className="flex items-center px-3 py-1.5 text-sm text-red-600 hover:text-red-700 hover:bg-red-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <StopIcon className="w-4 h-4 mr-1" />
            停止
          </button>
        )}

        {onRestart && canRestart && (
          <button
            onClick={() => onRestart(container.name)}
            disabled={isLoading}
            className="flex items-center px-3 py-1.5 text-sm text-orange-600 hover:text-orange-700 hover:bg-orange-50 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <ArrowPathIcon className="w-4 h-4 mr-1" />
            重启
          </button>
        )}
      </div>
    </div>
  );
};
