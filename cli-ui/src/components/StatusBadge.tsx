import React from 'react';
import { ContainerInfo } from '../types';

interface StatusBadgeProps {
  status: ContainerInfo['status'];
  className?: string;
}

export const StatusBadge: React.FC<StatusBadgeProps> = ({ status, className = '' }) => {
  const getStatusColor = (): string => {
    switch (status) {
      case 'running':
        return 'bg-green-100 text-green-800 border-green-200';
      case 'stopped':
      case 'exited':
        return 'bg-gray-100 text-gray-800 border-gray-200';
      case 'starting':
      case 'restarting':
        return 'bg-blue-100 text-blue-800 border-blue-200';
      case 'stopping':
        return 'bg-yellow-100 text-yellow-800 border-yellow-200';
      case 'paused':
        return 'bg-orange-100 text-orange-800 border-orange-200';
      case 'dead':
        return 'bg-red-100 text-red-800 border-red-200';
      case 'unknown':
      default:
        return 'bg-gray-100 text-gray-600 border-gray-200';
    }
  };

  const getStatusIcon = (): string => {
    switch (status) {
      case 'running':
        return '🟢';
      case 'stopped':
      case 'exited':
        return '⚫';
      case 'starting':
      case 'restarting':
        return '🔵';
      case 'stopping':
        return '🟡';
      case 'paused':
        return '🟠';
      case 'dead':
        return '🔴';
      case 'unknown':
      default:
        return '⚪';
    }
  };

  const getStatusText = (): string => {
    switch (status) {
      case 'running':
        return '运行中';
      case 'stopped':
        return '已停止';
      case 'exited':
        return '已退出';
      case 'starting':
        return '启动中';
      case 'restarting':
        return '重启中';
      case 'stopping':
        return '停止中';
      case 'paused':
        return '已暂停';
      case 'dead':
        return '已死亡';
      case 'unknown':
      default:
        return '未知';
    }
  };

  return (
    <span
      className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border ${getStatusColor()} ${className}`}
    >
      <span className="mr-1">{getStatusIcon()}</span>
      {getStatusText()}
    </span>
  );
};
