import { useState, useCallback, useEffect, useRef, lazy, Suspense } from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { listen } from '@tauri-apps/api/event';
import { MainLayout } from './components/MainLayout';
import WelcomeSetupModal from './components/WelcomeSetupModal';
import ErrorBoundary from './components/ErrorBoundary';
import { AppProvider } from './context/AppContext';
import { LogEntry, DEFAULT_LOG_CONFIG, LogConfig } from './types';
import { ConfigManager, DialogManager, DuckCliManager, FileSystemManager, ProcessManager } from './utils/tauri';
import './App.css';

// 懒加载页面组件
const OverviewPage = lazy(() => import('./pages/OverviewPage').then(m => ({ default: m.OverviewPage })));
const DeployPage = lazy(() => import('./pages/DeployPage').then(m => ({ default: m.DeployPage })));
const ContainersPage = lazy(() => import('./pages/ContainersPage').then(m => ({ default: m.ContainersPage })));
const BackupPage = lazy(() => import('./pages/BackupPage').then(m => ({ default: m.BackupPage })));
const LogsPage = lazy(() => import('./pages/LogsPage').then(m => ({ default: m.LogsPage })));

function App() {
  // 工作目录状态
  const [workingDirectory, setWorkingDirectory] = useState<string | null>(null);
  const [isDirectoryValid, setIsDirectoryValid] = useState(false);
  const [showWelcomeModal, setShowWelcomeModal] = useState(false);
  
  // 日志状态 - 使用循环缓冲区
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logConfig] = useState<LogConfig>(DEFAULT_LOG_CONFIG);
  const [totalLogCount, setTotalLogCount] = useState(0); // 总日志数量统计
  const [isInitialized, setIsInitialized] = useState(false); // 初始化状态标记
  const [isAppLoading, setIsAppLoading] = useState(true); // 应用启动加载状态
  
  // 当前执行状态
  const [isExecuting, setIsExecuting] = useState(false);
  
  // 使用 useRef 避免循环依赖
  const logsRef = useRef<LogEntry[]>([]);

  // 同步 logs 状态到 ref
  useEffect(() => {
    logsRef.current = logs;
  }, [logs]);

  // 智能日志管理 - 循环缓冲区实现
  const manageLogBuffer = useCallback((newLogs: LogEntry[]) => {
    setLogs(currentLogs => {
      const allLogs = [...currentLogs, ...newLogs];
      
      // 检查是否需要清理
      if (allLogs.length > logConfig.maxEntries) {
        const excessCount = allLogs.length - logConfig.maxEntries;
        const trimCount = Math.max(excessCount, logConfig.trimBatchSize);
        
        // 保留最新的日志条目
        const trimmedLogs = allLogs.slice(trimCount);
        
        console.log(`日志缓冲区清理: 删除 ${trimCount} 条旧记录, 保留 ${trimmedLogs.length} 条`);
        
        return trimmedLogs;
      }
      
      return allLogs;
    });
  }, [logConfig.maxEntries, logConfig.trimBatchSize]);

  // 轻量级去重逻辑 - 只检查连续重复
  const shouldSkipDuplicate = useCallback((newMessage: string, newType: LogEntry['type']) => {
    const currentLogs = logsRef.current;
    if (currentLogs.length === 0) return false;
    
    // 只检查最后一条日志，避免连续重复（极端情况的保护）
    const lastLog = currentLogs[currentLogs.length - 1];
    return lastLog && 
      lastLog.message === newMessage && 
      lastLog.type === newType;
  }, []);

  // 添加日志条目 - 使用循环缓冲区
  const addLogEntry = useCallback((
    type: LogEntry['type'], 
    message: string, 
    command?: string, 
    args?: string[]
  ) => {
    // 过滤空消息
    if (!message.trim() && type !== 'command') return;
    
    // 只对相同类型的连续消息做去重，移除时间限制
    if (shouldSkipDuplicate(message, type)) return;
    
    const entry: LogEntry = {
      id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
      timestamp: new Date().toLocaleTimeString(),
      type,
      message,
      command,
      args
    };
    
    // 更新统计
    setTotalLogCount(prev => prev + 1);
    
    // 使用循环缓冲区管理
    manageLogBuffer([entry]);
  }, [shouldSkipDuplicate, manageLogBuffer]);

  // 导出所有日志
  const exportAllLogs = useCallback(async () => {
    try {
      // 检查是否有日志可导出
      if (logs.length === 0) {
        await DialogManager.showMessage('提示', '当前没有日志可导出', 'info');
        return false;
      }

      const timestamp = new Date().toISOString().slice(0, 19).replace(/:/g, '-');
      const filename = `duck-cli-logs-${timestamp}.txt`;
      
      console.log(`准备导出 ${logs.length} 条日志...`);
      
      const logContent = logs.map(log => {
        const prefix = `[${log.timestamp}] [${log.type.toUpperCase()}]`;
        if (log.type === 'command') {
          return `${prefix} $ ${log.command} ${log.args?.join(' ') || ''}`;
        }
        return `${prefix} ${log.message}`;
      }).join('\n');

      console.log(`日志内容长度: ${logContent.length} 字符`);

      // 打开保存对话框
      const savedPath = await DialogManager.saveFile('导出日志', filename);
      console.log('用户选择的保存路径:', savedPath);
      
      if (savedPath) {
        console.log(`开始写入文件: ${savedPath}`);
        const success = await FileSystemManager.writeTextFile(savedPath, logContent);
        console.log('文件写入结果:', success);
        
        if (success) {
          // 验证文件是否真的被创建
          const fileExists = await FileSystemManager.pathExists(savedPath);
          console.log('文件是否存在:', fileExists);
          
          if (fileExists) {
            await DialogManager.showMessage('成功', `日志已成功导出到:\n${savedPath}\n\n共导出 ${logs.length} 条日志记录`, 'info');
            
            // 添加导出成功的日志记录
            addLogEntry('success', `✅ 日志导出成功: ${savedPath} (${logs.length} 条记录)`);
            return true;
          } else {
            throw new Error('文件写入成功但文件不存在，可能是权限问题');
          }
        } else {
          throw new Error('文件写入失败');
        }
      } else {
        console.log('用户取消了文件保存操作');
        addLogEntry('info', '用户取消了日志导出操作');
        return false;
      }
    } catch (error) {
      console.error('Export logs failed:', error);
      await DialogManager.showMessage('错误', `日志导出失败:\n${error}`, 'error');
      addLogEntry('error', `❌ 日志导出失败: ${error}`);
      return false;
    }
  }, [logs, addLogEntry]);

  // 设置Tauri事件监听器 - 使用全局变量确保应用生命周期内只设置一次
  const addLogEntryRef = useRef(addLogEntry);
  
  // 同步最新的addLogEntry函数到ref
  useEffect(() => {
    addLogEntryRef.current = addLogEntry;
  }, [addLogEntry]);

  useEffect(() => {
    // 使用全局变量防止重复设置监听器
    if ((window as any).__duck_cli_listeners_setup) {
      return;
    }

    (window as any).__duck_cli_listeners_setup = true;

    let unlistenOutput: any;
    let unlistenError: any;
    let unlistenComplete: any;

    const setupEventListeners = async () => {
      try {
        // 监听CLI输出事件
        unlistenOutput = await listen('cli-output', (event) => {
          const output = event.payload as string;
          if (output.trim()) {
            addLogEntryRef.current('info', output.trim());
          }
        });

        // 监听CLI错误事件
        unlistenError = await listen('cli-error', (event) => {
          const error = event.payload as string;
          if (error.trim()) {
            addLogEntryRef.current('error', error.trim());
          }
        });

        // 监听CLI完成事件
        unlistenComplete = await listen('cli-complete', (event) => {
          const exitCode = event.payload as number;
          setIsExecuting(false);
          
          if (exitCode === 0) {
            addLogEntryRef.current('success', `命令执行完成 (退出码: ${exitCode})`);
          } else {
            addLogEntryRef.current('error', `命令执行失败 (退出码: ${exitCode})`);
          }
          
          // 添加分隔线
          addLogEntryRef.current('info', '─'.repeat(50));
        });

        // 保存清理函数到全局变量
        (window as any).__duck_cli_listeners_cleanup = () => {
          if (unlistenOutput) unlistenOutput();
          if (unlistenError) unlistenError();
          if (unlistenComplete) unlistenComplete();
          (window as any).__duck_cli_listeners_setup = false;
        };
      } catch (error) {
        console.error('设置事件监听器失败:', error);
        (window as any).__duck_cli_listeners_setup = false;
      }
    };

    setupEventListeners();

    // 清理函数
    return () => {
      // 不在组件卸载时清理全局监听器，让它们在应用生命周期内持续存在
    };
  }, []); // 空依赖数组，确保只注册一次

  // 处理工作目录变化
  const handleDirectoryChange = useCallback(async (directory: string | null, isValid: boolean) => {
    console.log('工作目录变更:', directory, '有效性:', isValid);
    
    const previousDirectory = workingDirectory;
    setWorkingDirectory(directory);
    setIsDirectoryValid(isValid);

    if (directory && isValid && directory !== previousDirectory) {
      // 保存工作目录配置
      try {
        await ConfigManager.setWorkingDirectory(directory);
        console.log('工作目录已保存到配置:', directory);
      } catch (error) {
        console.error('保存工作目录失败:', error);
        addLogEntry('warning', `⚠️ 保存工作目录失败: ${error}`);
      }
      
      // 立即设置目录，不阻塞界面
      addLogEntry('info', `📁 工作目录已设置: ${directory}`);
      
      // 将耗时的进程检查移到后台异步执行
      setTimeout(async () => {
        try {
          addLogEntry('info', '🔍 后台检查并清理冲突进程...');
          const checkResult = await ProcessManager.initializeProcessCheck(directory);
          
          if (checkResult.processCleanup.processes_found.length > 0) {
            addLogEntry('warning', `🧹 发现 ${checkResult.processCleanup.processes_found.length} 个冲突进程`);
            addLogEntry('success', `✅ 已清理 ${checkResult.processCleanup.processes_killed.length} 个进程`);
          }
          
          if (checkResult.databaseLocked) {
            addLogEntry('error', '⚠️ 数据库文件仍被锁定，请稍后重试');
            setIsDirectoryValid(false); // 临时禁用功能直到锁定解除
          } else {
            addLogEntry('success', checkResult.message);
          }
        } catch (error) {
          console.error('进程检查失败:', error);
          addLogEntry('warning', `⚠️ 进程检查失败: ${error}，但不影响正常使用`);
          // 进程检查失败不影响工作目录的有效性
        }
      }, 100); // 100ms 后执行，不阻塞界面
    }

    // 根据是否需要显示欢迎界面
    if (!directory || !isValid) {
      setShowWelcomeModal(true);
    } else {
      setShowWelcomeModal(false);
    }
  }, [workingDirectory, addLogEntry]);

  // 处理命令执行
  const handleCommandExecute = useCallback(async (command: string, args: string[]) => {
    // 防止重复执行
    if (isExecuting) {
      return;
    }
    
    addLogEntry('command', '', command, args);
    setIsExecuting(true);
    
    // 添加执行开始标记
    addLogEntry('info', `🚀 开始执行: ${command} ${args.join(' ')}`);
    
    try {
      // 真正执行Tauri命令，会触发事件监听器接收实时输出
      if (command === 'duck-cli' && workingDirectory) {
        await DuckCliManager.executeSmart(args, workingDirectory);
      }
    } catch (error) {
      addLogEntry('error', `❌ 命令执行失败: ${error}`);
      setIsExecuting(false); // 异常时手动重置状态
    }
    // 注意：setIsExecuting(false) 会在事件监听器的 cli-complete 事件中处理
  }, [addLogEntry, workingDirectory, isExecuting]);

  // 处理日志消息
  const handleLogMessage = useCallback((message: string, type: LogEntry['type']) => {
    addLogEntry(type, message);
  }, [addLogEntry]);

  // 清除日志
  const handleClearLogs = useCallback(() => {
    setLogs([]);
    setTotalLogCount(0);
    addLogEntry('info', '日志已清除');
  }, [addLogEntry]);

  // 应用初始化 - 只执行一次
  useEffect(() => {
    if (isInitialized) return;

    const initializeApp = async () => {
      console.log('开始初始化应用...');
      
      // 标记应用正在初始化，防止其他组件重复初始化
      (window as any).__duck_app_initializing = true;
      
      // 使用直接的状态更新避免循环
      const initEntry: LogEntry = {
        id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
        timestamp: new Date().toLocaleTimeString(),
        type: 'info',
        message: '🚀 Duck CLI GUI 已启动'
      };
      
      const configEntry: LogEntry = {
        id: (Date.now() + 1).toString() + Math.random().toString(36).substr(2, 9),
        timestamp: new Date().toLocaleTimeString(),
        type: 'info',
        message: `📊 日志管理: 最大 ${logConfig.maxEntries} 条，自动循环覆盖旧记录`
      };
      
      setLogs([initEntry, configEntry]);
      setTotalLogCount(2);
      
      try {
        // 检查是否已有保存的工作目录
        const savedDirectory = await ConfigManager.getWorkingDirectory();
        
        if (savedDirectory) {
          // 验证保存的目录
          const validation = await FileSystemManager.validateDirectory(savedDirectory);
          await handleDirectoryChange(savedDirectory, validation.valid);
        } else {
          setShowWelcomeModal(true);
        }
      } catch (error) {
        console.error('初始化失败:', error);
        setShowWelcomeModal(true);
      }
      
      // 标记应用初始化完成
      (window as any).__duck_app_initialized = true;
      (window as any).__duck_app_initializing = false;
      
      setIsInitialized(true);
      setIsAppLoading(false); // 停止加载状态
      console.log('应用初始化完成');
    };

    initializeApp();
  }, [isInitialized, logConfig.maxEntries, handleDirectoryChange]);

  // 处理更改目录按钮点击
  const handleChangeDirectoryClick = useCallback(() => {
    setShowWelcomeModal(true);
  }, []);

  return (
    <BrowserRouter>
      <div className="h-screen flex flex-col bg-gray-100">
        {/* 应用启动加载界面 */}
        {isAppLoading && (
          <div className="fixed inset-0 bg-white bg-opacity-90 flex items-center justify-center z-50">
            <div className="text-center">
              <div className="text-6xl mb-4">🦆</div>
              <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500 mx-auto mb-4"></div>
              <h2 className="text-xl font-semibold text-gray-900 mb-2">Duck CLI GUI</h2>
              <p className="text-gray-600">正在启动应用...</p>
            </div>
          </div>
        )}

        {/* 主应用界面 */}
        {!isAppLoading && (
          <>
            {/* 路由配置 */}
            <Routes>
              <Route path="/" element={<MainLayout onChangeDirectory={handleChangeDirectoryClick} />}>
                {/* 默认重定向到概览页 */}
                <Route index element={<Navigate to="/overview" replace />} />
                
                {/* 各个功能页面 */}
                <Route 
                  path="overview" 
                  element={
                    <Suspense fallback={<LoadingFallback />}>
                      <OverviewPage />
                    </Suspense>
                  } 
                />
                <Route 
                  path="deploy" 
                  element={
                    <Suspense fallback={<LoadingFallback />}>
                      <DeployPage />
                    </Suspense>
                  } 
                />
                <Route 
                  path="containers" 
                  element={
                    <Suspense fallback={<LoadingFallback />}>
                      <ContainersPage />
                    </Suspense>
                  } 
                />
                <Route 
                  path="backup" 
                  element={
                    <Suspense fallback={<LoadingFallback />}>
                      <BackupPage />
                    </Suspense>
                  } 
                />
                <Route 
                  path="logs" 
                  element={
                    <Suspense fallback={<LoadingFallback />}>
                      <LogsPage />
                    </Suspense>
                  } 
                />
                
                {/* 404 重定向 */}
                <Route path="*" element={<Navigate to="/overview" replace />} />
              </Route>
            </Routes>
          </>
        )}

        {/* 执行状态指示器 */}
        {isExecuting && !isAppLoading && (
          <div className="fixed bottom-4 right-4 bg-blue-600 text-white px-4 py-2 rounded-lg shadow-lg flex items-center space-x-2">
            <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white"></div>
            <span className="text-sm font-medium">正在执行命令...</span>
          </div>
        )}

        {/* 欢迎设置弹窗 */}
        {showWelcomeModal && !isAppLoading && (
          <WelcomeSetupModal
            isOpen={showWelcomeModal}
            onComplete={async (directory: string) => {
              // 验证目录
              const validation = await FileSystemManager.validateDirectory(directory);
              await handleDirectoryChange(directory, validation.valid);
              setShowWelcomeModal(false);
            }}
            onSkip={() => setShowWelcomeModal(false)}
          />
        )}
      </div>
    </BrowserRouter>
  );
}

// 加载中占位组件
const LoadingFallback: React.FC = () => (
  <div className="flex items-center justify-center h-full">
    <div className="text-center">
      <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500 mx-auto mb-4"></div>
      <p className="text-gray-600">加载中...</p>
    </div>
  </div>
);
}

export default function AppWithErrorBoundary() {
  return (
    <ErrorBoundary>
      <AppProvider>
        <App />
      </AppProvider>
    </ErrorBoundary>
  );
}
