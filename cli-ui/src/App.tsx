import { useCallback, useState } from 'react';
import WorkingDirectoryBar from './components/WorkingDirectoryBar';
import OperationPanel from './components/OperationPanel';
import TerminalWindow from './components/TerminalWindow';
import WelcomeSetupModal from './components/WelcomeSetupModal';
import ErrorBoundary from './components/ErrorBoundary';
import { LogEntry } from './types';
import { ConfigManager, DialogManager, FileSystemManager, ProcessManager } from './utils/tauri';
import { useAppStore } from './store/appStore';
import { useAppInit } from './hooks/useAppInit';
import { useCliEvents } from './hooks/useCliEvents';
import { cliGateway } from './services/cliGateway';
import './App.css';

function App() {
  const workingDirectory = useAppStore((state) => state.workingDirectory);
  const showWelcomeModal = useAppStore((state) => state.showWelcomeModal);
  const isAppLoading = useAppStore((state) => state.isAppLoading);
  const isExecuting = useAppStore((state) => state.isExecuting);
  const logs = useAppStore((state) => state.logs);
  const logConfig = useAppStore((state) => state.logConfig);
  const totalLogCount = useAppStore((state) => state.totalLogCount);
  const setExecuting = useAppStore((state) => state.setExecuting);
  const setShowWelcomeModal = useAppStore((state) => state.setShowWelcomeModal);
  const setWorkingDirectory = useAppStore((state) => state.setWorkingDirectory);
  const setValidationState = useAppStore((state) => state.setValidationState);
  const addLog = useAppStore((state) => state.addLog);
  const clearLogs = useAppStore((state) => state.clearLogs);
  const [isRechecking, setIsRechecking] = useState(false);

  // 统一事件监听
  useCliEvents();

  // 处理工作目录变更、验证与进程检查
  const validateAndApplyDirectory = useCallback(async (directory: string) => {
    setValidationState('validating');
    setWorkingDirectory({ path: directory });

    try {
      const validation = await FileSystemManager.validateDirectory(directory);

      if (!validation.valid) {
        setWorkingDirectory({
          path: directory,
          isValid: false,
          validationState: 'invalid',
          error: validation.error,
        });
        setShowWelcomeModal(true);
        addLog('warning', validation.error || '目录验证失败');
        return;
      }

      setWorkingDirectory({
        path: directory,
        isValid: true,
        validationState: 'valid',
        error: undefined,
      });
      setShowWelcomeModal(false);
      addLog('info', `📁 工作目录已设置: ${directory}`);

      await ConfigManager.setWorkingDirectory(directory);

      try {
        addLog('info', '🔍 检查并清理冲突进程...');
        const checkResult = await ProcessManager.initializeProcessCheck(directory);

        if (checkResult.processCleanup.processes_found.length > 0) {
          addLog('warning', `🧹 发现 ${checkResult.processCleanup.processes_found.length} 个冲突进程`);
          addLog('success', `✅ 已清理 ${checkResult.processCleanup.processes_killed.length} 个进程`);
        }

        if (checkResult.databaseLocked) {
          addLog('error', '⚠️ 数据库文件仍被锁定，请稍后重试');
          setWorkingDirectory({
            isValid: false,
            validationState: 'invalid',
            error: '数据库文件被锁定',
          });
          setShowWelcomeModal(true);
        } else {
          addLog('success', checkResult.message);
        }
      } catch (error) {
        addLog('warning', `⚠️ 进程检查失败: ${error}，但不影响正常使用`);
      }
    } catch (error) {
      setWorkingDirectory({
        path: directory,
        isValid: false,
        validationState: 'invalid',
        error: String(error),
      });
      setShowWelcomeModal(true);
      addLog('error', `目录处理失败: ${error}`);
    }
  }, [addLog, setShowWelcomeModal, setValidationState, setWorkingDirectory]);

  // 初始化流程
  useAppInit(validateAndApplyDirectory);

  // 导出所有日志
  const exportAllLogs = useCallback(async () => {
    try {
      if (logs.length === 0) {
        await DialogManager.showMessage('提示', '当前没有日志可导出', 'info');
        return false;
      }

      const timestamp = new Date().toISOString().slice(0, 19).replace(/:/g, '-');
      const filename = `duck-cli-logs-${timestamp}.txt`;

      const logContent = logs.map(log => {
        const prefix = `[${log.timestamp}] [${log.type.toUpperCase()}]`;
        if (log.type === 'command') {
          return `${prefix} $ ${log.command} ${log.args?.join(' ') || ''}`;
        }
        return `${prefix} ${log.message}`;
      }).join('\n');

      const savedPath = await DialogManager.saveFile('导出日志', filename);
      
      if (savedPath) {
        const success = await FileSystemManager.writeTextFile(savedPath, logContent);
        
        if (success) {
          const fileExists = await FileSystemManager.pathExists(savedPath);
          
          if (fileExists) {
            await DialogManager.showMessage('成功', `日志已成功导出到:\n${savedPath}\n\n共导出 ${logs.length} 条日志记录`, 'info');
            addLog('success', `✅ 日志导出成功: ${savedPath} (${logs.length} 条记录)`);
            return true;
          } else {
            throw new Error('文件写入成功但文件不存在，可能是权限问题');
          }
        } else {
          throw new Error('文件写入失败');
        }
      } else {
        addLog('info', '用户取消了日志导出操作');
        return false;
      }
    } catch (error) {
      await DialogManager.showMessage('错误', `日志导出失败:\n${error}`, 'error');
      addLog('error', `❌ 日志导出失败: ${error}`);
      return false;
    }
  }, [logs, addLog]);

  // 处理工作目录选择
  const handleDirectorySelect = useCallback(async () => {
    const selectedPath = await DialogManager.selectDirectory();
    if (selectedPath) {
      await validateAndApplyDirectory(selectedPath);
    }
  }, [validateAndApplyDirectory]);

  // 手动重新检测进程/锁
  const handleRecheck = useCallback(async () => {
    if (!workingDirectory.path) {
      addLog('warning', '请先选择工作目录');
      return;
    }
    setIsRechecking(true);
    try {
      addLog('info', '🔍 重新检测进程与数据库锁状态...');
      const checkResult = await ProcessManager.initializeProcessCheck(workingDirectory.path);

      if (checkResult.processCleanup.processes_found.length > 0) {
        addLog('warning', `🧹 发现 ${checkResult.processCleanup.processes_found.length} 个冲突进程`);
        addLog('success', `✅ 已清理 ${checkResult.processCleanup.processes_killed.length} 个进程`);
      }

      if (checkResult.databaseLocked) {
        addLog('error', '⚠️ 数据库文件仍被锁定，请稍后重试');
      } else {
        addLog('success', checkResult.message);
      }
    } catch (error) {
      addLog('error', `检测失败: ${error}`);
    } finally {
      setIsRechecking(false);
    }
  }, [addLog, workingDirectory.path]);

  // 处理命令执行
  const handleCommandExecute = useCallback(async (command: string, args: string[]) => {
    if (isExecuting) {
      return;
    }

    if (!workingDirectory.path || !workingDirectory.isValid) {
      addLog('warning', '请先设置有效的工作目录');
      return;
    }
    
    const commandId = `${command}-${Date.now()}`;
    addLog('command', '', command, args);
    setExecuting(true);
    addLog('info', `🚀 开始执行: ${command} ${args.join(' ')} [${commandId}]`);
    
    try {
      if (command === 'duck-cli') {
        await cliGateway.execute(args, { workingDirectory: workingDirectory.path, commandId });
      }
    } catch (error) {
      addLog('error', `❌ 命令执行失败: ${error}`);
      setExecuting(false);
    }
    // setExecuting(false) 将由 cli-complete 事件处理
  }, [addLog, isExecuting, setExecuting, workingDirectory.path, workingDirectory.isValid]);

  // 处理日志消息
  const handleLogMessage = useCallback((message: string, type: LogEntry['type']) => {
    addLog(type, message);
  }, [addLog]);

  // 清除日志
  const handleClearLogs = useCallback(() => {
    clearLogs();
    addLog('info', '日志已清除');
  }, [addLog, clearLogs]);

  return (
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
          {/* 顶部工作目录栏 */}
          <WorkingDirectoryBar 
            workingDirectory={workingDirectory.path}
            validationState={workingDirectory.validationState}
            validationError={workingDirectory.error}
            onSelectDirectory={handleDirectorySelect}
            onRecheck={handleRecheck}
            isRechecking={isRechecking}
          />

          {/* 主内容区域 */}
          <div className="flex-1 flex flex-col min-h-0">
            {/* 上半部分：操作面板 - 使用自适应高度 */}
            <div className="flex-shrink-0 overflow-auto">
              <OperationPanel
                workingDirectory={workingDirectory.path}
                isDirectoryValid={workingDirectory.isValid}
                onCommandExecute={handleCommandExecute}
                onLogMessage={handleLogMessage}
              />
            </div>
            
            {/* 下半部分：终端窗口 - 占用剩余空间 */}
            <div className="flex-1 border-t border-gray-200 min-h-0">
              <TerminalWindow
                logs={logs}
                onClearLogs={handleClearLogs}
                isEnabled={workingDirectory.isValid}
                totalLogCount={totalLogCount}
                maxLogEntries={logConfig.maxEntries}
                onExportLogs={exportAllLogs}
              />
            </div>
          </div>
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
            await validateAndApplyDirectory(directory);
            setShowWelcomeModal(false);
          }}
          onSkip={() => setShowWelcomeModal(false)}
        />
      )}
    </div>
  );
}

export default function AppWithErrorBoundary() {
  return (
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  );
}
