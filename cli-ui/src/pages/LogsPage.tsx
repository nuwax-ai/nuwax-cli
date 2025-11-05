import React, { useState, useMemo } from 'react';
import { useApp } from '../context/AppContext';
import { LogTerminal } from '../components/LogTerminal';
import { LogEntry } from '../types';
import {
  TrashIcon,
  ArrowDownTrayIcon,
  MagnifyingGlassIcon,
  FunnelIcon,
} from '@heroicons/react/24/outline';

export const LogsPage: React.FC = () => {
  const { logs, clearLogs, logConfig } = useApp();
  const [searchQuery, setSearchQuery] = useState('');
  const [filterType, setFilterType] = useState<LogEntry['type'] | 'all'>('all');

  // 过滤日志
  const filteredLogs = useMemo(() => {
    let result = logs;

    // 按类型过滤
    if (filterType !== 'all') {
      result = result.filter((log) => log.type === filterType);
    }

    // 按搜索关键词过滤
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter(
        (log) =>
          log.message.toLowerCase().includes(query) ||
          log.command?.toLowerCase().includes(query) ||
          log.args?.some((arg) => arg.toLowerCase().includes(query))
      );
    }

    return result;
  }, [logs, filterType, searchQuery]);

  // 导出日志
  const handleExportLogs = () => {
    const logText = filteredLogs
      .map((log) => {
        const timestamp = new Date(log.timestamp).toLocaleString('zh-CN');
        const command = log.command ? `[${log.command} ${log.args?.join(' ') || ''}] ` : '';
        return `[${timestamp}] [${log.type.toUpperCase()}] ${command}${log.message}`;
      })
      .join('\n');

    const blob = new Blob([logText], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `logs-${new Date().toISOString().replace(/[:.]/g, '-')}.txt`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  // 清除日志
  const handleClearLogs = () => {
    const confirmed = window.confirm('确定要清除所有日志吗？此操作不可恢复。');
    if (confirmed) {
      clearLogs();
    }
  };

  // 统计信息
  const stats = useMemo(() => {
    const total = logs.length;
    const info = logs.filter((log) => log.type === 'info').length;
    const success = logs.filter((log) => log.type === 'success').length;
    const error = logs.filter((log) => log.type === 'error').length;
    const warning = logs.filter((log) => log.type === 'warning').length;
    const command = logs.filter((log) => log.type === 'command').length;

    return { total, info, success, error, warning, command };
  }, [logs]);

  return (
    <div className="p-6 space-y-6">
      {/* 统计信息 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">日志统计</h2>
        <div className="grid grid-cols-2 md:grid-cols-6 gap-4">
          <div className="text-center">
            <p className="text-2xl font-bold text-gray-900">{stats.total}</p>
            <p className="text-sm text-gray-500">总计</p>
          </div>
          <div className="text-center">
            <p className="text-2xl font-bold text-gray-600">{stats.info}</p>
            <p className="text-sm text-gray-500">信息</p>
          </div>
          <div className="text-center">
            <p className="text-2xl font-bold text-green-600">{stats.success}</p>
            <p className="text-sm text-gray-500">成功</p>
          </div>
          <div className="text-center">
            <p className="text-2xl font-bold text-red-600">{stats.error}</p>
            <p className="text-sm text-gray-500">错误</p>
          </div>
          <div className="text-center">
            <p className="text-2xl font-bold text-yellow-600">{stats.warning}</p>
            <p className="text-sm text-gray-500">警告</p>
          </div>
          <div className="text-center">
            <p className="text-2xl font-bold text-blue-600">{stats.command}</p>
            <p className="text-sm text-gray-500">命令</p>
          </div>
        </div>

        <div className="mt-4 p-3 bg-gray-50 rounded-lg">
          <p className="text-sm text-gray-600">
            💡 缓冲区配置：最多保留 {logConfig.maxEntries.toLocaleString()} 条日志，
            超出时自动清理 {logConfig.trimBatchSize.toLocaleString()} 条旧日志
          </p>
        </div>
      </div>

      {/* 操作栏 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <div className="flex flex-col md:flex-row md:items-center md:justify-between space-y-4 md:space-y-0">
          {/* 搜索框 */}
          <div className="flex-1 max-w-md">
            <div className="relative">
              <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                <MagnifyingGlassIcon className="h-5 w-5 text-gray-400" />
              </div>
              <input
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="搜索日志内容..."
                className="block w-full pl-10 pr-3 py-2 border border-gray-300 rounded-lg focus:ring-blue-500 focus:border-blue-500 text-sm"
              />
            </div>
          </div>

          {/* 过滤器和操作按钮 */}
          <div className="flex items-center space-x-3">
            {/* 类型过滤 */}
            <div className="flex items-center space-x-2">
              <FunnelIcon className="h-5 w-5 text-gray-400" />
              <select
                value={filterType}
                onChange={(e) => setFilterType(e.target.value as LogEntry['type'] | 'all')}
                className="block pl-3 pr-10 py-2 text-sm border border-gray-300 rounded-lg focus:ring-blue-500 focus:border-blue-500"
              >
                <option value="all">全部类型</option>
                <option value="info">信息</option>
                <option value="success">成功</option>
                <option value="error">错误</option>
                <option value="warning">警告</option>
                <option value="command">命令</option>
              </select>
            </div>

            {/* 导出按钮 */}
            <button
              onClick={handleExportLogs}
              disabled={filteredLogs.length === 0}
              className="flex items-center px-4 py-2 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <ArrowDownTrayIcon className="w-4 h-4 mr-1" />
              导出
            </button>

            {/* 清除按钮 */}
            <button
              onClick={handleClearLogs}
              disabled={logs.length === 0}
              className="flex items-center px-4 py-2 text-sm text-red-600 hover:text-red-700 hover:bg-red-50 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <TrashIcon className="w-4 h-4 mr-1" />
              清除
            </button>
          </div>
        </div>

        {/* 过滤结果提示 */}
        {(searchQuery || filterType !== 'all') && (
          <div className="mt-4 p-3 bg-blue-50 border border-blue-200 rounded-lg">
            <p className="text-sm text-blue-700">
              {filteredLogs.length === logs.length ? (
                <>显示全部 {logs.length} 条日志</>
              ) : (
                <>
                  已过滤：显示 {filteredLogs.length} / {logs.length} 条日志
                  {searchQuery && <> (搜索: "{searchQuery}")</>}
                  {filterType !== 'all' && <> (类型: {filterType})</>}
                </>
              )}
            </p>
          </div>
        )}
      </div>

      {/* 日志终端 */}
      <div className="bg-white rounded-lg border border-gray-200 p-6">
        <h2 className="text-xl font-semibold text-gray-900 mb-4">日志输出</h2>
        <LogTerminal logs={filteredLogs} maxHeight="600px" />

        {filteredLogs.length === 0 && logs.length > 0 && (
          <div className="text-center py-8 text-gray-500">
            没有符合条件的日志
          </div>
        )}

        {logs.length === 0 && (
          <div className="text-center py-8 text-gray-500">
            暂无日志，执行命令后将在此显示
          </div>
        )}
      </div>
    </div>
  );
};
