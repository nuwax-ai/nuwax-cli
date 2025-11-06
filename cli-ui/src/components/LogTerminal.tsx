import React, { useEffect, useRef, memo } from 'react';
import { LogEntry } from '../types';

interface LogTerminalProps {
  logs: LogEntry[];
  className?: string;
  autoScroll?: boolean;
  maxHeight?: string;
}

// 使用 React.memo 优化渲染性能
export const LogTerminal: React.FC<LogTerminalProps> = memo(({
  logs,
  className = '',
  autoScroll = true,
  maxHeight = '400px',
}) => {
  const terminalRef = useRef<HTMLDivElement>(null);

  // 自动滚动到底部
  useEffect(() => {
    if (autoScroll && terminalRef.current) {
      terminalRef.current.scrollTop = terminalRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  const getLogColor = (type: LogEntry['type']): string => {
    switch (type) {
      case 'success':
        return 'text-green-600';
      case 'error':
        return 'text-red-600';
      case 'warning':
        return 'text-yellow-600';
      case 'command':
        return 'text-blue-600 font-semibold';
      case 'info':
      default:
        return 'text-gray-700';
    }
  };

  const getLogIcon = (type: LogEntry['type']): string => {
    switch (type) {
      case 'success':
        return '✅';
      case 'error':
        return '❌';
      case 'warning':
        return '⚠️';
      case 'command':
        return '🚀';
      case 'info':
      default:
        return 'ℹ️';
    }
  };

  const formatTimestamp = (timestamp: string): string => {
    const date = new Date(timestamp);
    return date.toLocaleTimeString('zh-CN', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  };

  return (
    <div
      ref={terminalRef}
      className={`bg-gray-900 rounded-lg p-4 font-mono text-sm overflow-y-auto ${className}`}
      style={{ maxHeight }}
    >
      {logs.length === 0 ? (
        <div className="text-gray-500 text-center py-8">暂无日志</div>
      ) : (
        <div className="space-y-1">
          {/* 
            性能优化说明：
            - 使用循环缓冲区限制日志数量（在 AppContext 中实现）
            - 当日志数量超过 1000 条时，考虑使用虚拟滚动库（如 react-window）
            - 当前实现已通过 React.memo 优化渲染性能
          */}
          {logs.map((log) => (
            <div key={log.id} className="flex items-start space-x-2">
              <span className="text-gray-500 text-xs flex-shrink-0">
                [{formatTimestamp(log.timestamp)}]
              </span>
              <span className="flex-shrink-0">{getLogIcon(log.type)}</span>
              <span className={`flex-1 break-words ${getLogColor(log.type)}`}>
                {log.command && (
                  <span className="text-blue-400">
                    $ {log.command} {log.args?.join(' ')}
                    <br />
                  </span>
                )}
                {log.message}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}, (prevProps, nextProps) => {
  // 自定义比较函数：只在 logs 数组长度或最后一条日志变化时重新渲染
  return (
    prevProps.logs.length === nextProps.logs.length &&
    prevProps.logs[prevProps.logs.length - 1]?.id === nextProps.logs[nextProps.logs.length - 1]?.id &&
    prevProps.className === nextProps.className &&
    prevProps.autoScroll === nextProps.autoScroll &&
    prevProps.maxHeight === nextProps.maxHeight
  );
});
