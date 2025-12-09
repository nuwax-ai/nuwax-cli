import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { 
  CommandLineIcon, 
  TrashIcon,
  DocumentTextIcon,
  ArrowDownTrayIcon,
  ChevronDownIcon,
  PauseIcon,
  PlayIcon,
  ChartBarIcon
} from '@heroicons/react/24/outline';
import { FixedSizeList as List, ListOnScrollProps } from 'react-window';
import { DialogManager } from '../utils/tauri';

interface LogEntry {
  id: string;
  timestamp: string;
  type: 'info' | 'success' | 'error' | 'warning' | 'command';
  message: string;
  command?: string;
  args?: string[];
}

interface TerminalWindowProps {
  logs: LogEntry[];
  onClearLogs: () => void;
  isEnabled: boolean;
  totalLogCount: number;           // 总日志数量统计
  maxLogEntries: number;           // 最大日志条目数
  onExportLogs: () => Promise<boolean>; // 导出日志函数
}

const ROW_HEIGHT = 24;

const TerminalWindow: React.FC<TerminalWindowProps> = ({ 
  logs, 
  onClearLogs, 
  isEnabled,
  totalLogCount,
  maxLogEntries,
  onExportLogs
}) => {
  const [autoScroll, setAutoScroll] = useState(true);
  const [listHeight, setListHeight] = useState(320);
  const containerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<List>(null);
  const userInteractedRef = useRef(false);
  const isAutoScrollingRef = useRef(false);

  // 监听容器尺寸变化以适配虚拟列表高度
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const update = () => {
      const rect = el.getBoundingClientRect();
      setListHeight(rect.height);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // 新日志时自动滚动到底部
  useEffect(() => {
    if (!autoScroll || logs.length === 0 || !listRef.current) return;
    isAutoScrollingRef.current = true;
    listRef.current.scrollToItem(logs.length - 1);
    setTimeout(() => {
      isAutoScrollingRef.current = false;
    }, 50);
  }, [logs, autoScroll]);

  const handleScroll = (props: ListOnScrollProps) => {
    if (isAutoScrollingRef.current) return;
    const { scrollOffset, scrollDirection, scrollUpdateWasRequested } = props;
    if (scrollUpdateWasRequested) return;

    // 估算是否在底部
    const maxOffset = Math.max(0, logs.length * ROW_HEIGHT - listHeight);
    const isAtBottom = scrollOffset >= maxOffset - ROW_HEIGHT;
    if (!isAtBottom && scrollDirection === 'forward' && userInteractedRef.current) {
      setAutoScroll(false);
    }
  };

  const handleUserInteraction = () => {
    userInteractedRef.current = true;
    setTimeout(() => { userInteractedRef.current = false; }, 500);
  };

  const scrollToBottom = () => {
    if (!listRef.current) return;
    isAutoScrollingRef.current = true;
    listRef.current.scrollToItem(logs.length - 1);
    setAutoScroll(true);
    setTimeout(() => { isAutoScrollingRef.current = false; }, 50);
  };

  const exportLogs = async () => {
    try {
      const success = await onExportLogs();
      if (success) {
        console.log('日志导出成功');
      }
    } catch (error) {
      console.error('Export logs failed:', error);
      await DialogManager.showMessage('错误', '导出失败', 'error');
    }
  };

  const currentLogs = logs.length;
  const percentage = Math.round((currentLogs / maxLogEntries) * 100);

  const renderRow = ({ index, style }: { index: number; style: React.CSSProperties }) => {
    const log = logs[index];
    return (
      <div style={style} className="flex items-start space-x-2 px-4">
        <span className="text-gray-500 text-xs flex-shrink-0 mt-0.5">
          {log.timestamp}
        </span>
        <div className="flex-1 min-w-0">
          {log.type === 'command' ? (
            <div className="text-blue-400 text-sm">
              <span className="text-gray-500">$</span> {log.command} {log.args?.join(' ')}
            </div>
          ) : (
            <div className={
              log.type === 'error' ? 'text-red-400 text-sm' :
              log.type === 'success' ? 'text-green-400 text-sm' :
              log.type === 'warning' ? 'text-yellow-400 text-sm' :
              'text-gray-300 text-sm'
            }>
              <span className="mr-2">
                {log.type === 'error' ? '✗' :
                 log.type === 'success' ? '✓' :
                 log.type === 'warning' ? '⚠' : 'ℹ'}
              </span>
              {log.message}
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="bg-white border-t border-gray-200 flex flex-col h-full">
      {/* 终端标题栏 */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-200 bg-gray-50">
        <div className="flex items-center space-x-2">
          <CommandLineIcon className="h-4 w-4 text-gray-500" />
          <span className="text-sm font-medium text-gray-700">终端输出</span>
          {!isEnabled && (
            <span className="text-xs text-yellow-600 bg-yellow-100 px-2 py-1 rounded">
              工作目录无效
            </span>
          )}
          
          {/* 日志统计信息 */}
          <div className="flex items-center space-x-2 text-xs text-gray-500">
            <span className="flex items-center space-x-1">
              <ChartBarIcon className="h-3 w-3" />
              <span>显示: {currentLogs}</span>
            </span>
            <span>总计: {totalLogCount}</span>
            <span className={`px-2 py-1 rounded ${
              percentage > 90 ? 'bg-red-100 text-red-700' :
              percentage > 70 ? 'bg-yellow-100 text-yellow-700' :
              'bg-green-100 text-green-700'
            }`}>
              缓冲区: {percentage}%
            </span>
          </div>
        </div>

        <div className="flex items-center space-x-2">
          {/* 自动滚动按钮 */}
          <button
            onClick={scrollToBottom}
            disabled={logs.length === 0}
            className={`flex items-center space-x-1 px-2 py-1 rounded text-xs transition-colors ${
              autoScroll 
                ? 'bg-green-100 text-green-700 hover:bg-green-200' 
                : 'bg-gray-100 text-gray-600 hover:bg-gray-200'
            } disabled:opacity-50 disabled:cursor-not-allowed`}
            title={autoScroll ? "自动滚动已开启，点击滚动到底部" : "自动滚动已暂停，点击恢复并滚动到底部"}
          >
            {autoScroll ? (
              <PlayIcon className="h-3 w-3" />
            ) : (
              <PauseIcon className="h-3 w-3" />
            )}
            <ChevronDownIcon className="h-3 w-3" />
            <span>{autoScroll ? "自动滚动" : "手动模式"}</span>
          </button>

          {/* 导出日志 */}
          <button
            onClick={exportLogs}
            disabled={logs.length === 0}
            className="p-1 text-gray-500 hover:text-gray-700 disabled:opacity-50 disabled:cursor-not-allowed"
            title="导出日志"
          >
            <ArrowDownTrayIcon className="h-4 w-4" />
          </button>

          {/* 清除日志 */}
          <button
            onClick={onClearLogs}
            disabled={logs.length === 0}
            className="p-1 text-gray-500 hover:text-red-600 disabled:opacity-50 disabled:cursor-not-allowed"
            title="清除日志"
          >
            <TrashIcon className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* 终端内容区域 */}
      <div 
        ref={containerRef}
        onMouseDown={handleUserInteraction}
        onWheel={handleUserInteraction}
        onKeyDown={handleUserInteraction}
        className="flex-1 overflow-hidden bg-gray-900 text-green-400 font-mono text-sm"
        style={{ minHeight: '300px' }}
      >
        {logs.length === 0 ? (
          <div className="flex items-center justify-center h-full text-gray-500">
            <div className="text-center">
              <DocumentTextIcon className="h-12 w-12 mx-auto mb-2 opacity-50" />
              <p className="text-sm">暂无日志信息</p>
              <p className="text-xs mt-1">执行操作后会在此显示输出</p>
              <p className="text-xs mt-2 text-gray-400">
                💡 日志管理: 最大 {maxLogEntries} 条，超出时自动覆盖最早记录
              </p>
            </div>
          </div>
        ) : (
          <List
            ref={listRef}
            height={listHeight}
            itemCount={logs.length}
            itemSize={ROW_HEIGHT}
            width="100%"
            onScroll={handleScroll}
            className="outline-none"
          >
            {renderRow}
          </List>
        )}
      </div>
    </div>
  );
};

export default TerminalWindow;