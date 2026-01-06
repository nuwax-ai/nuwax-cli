import { useEffect } from 'react';
import { ConfigManager } from '../utils/tauri';
import { useAppStore } from '../store/appStore';

type DirectoryInitializer = (path: string) => Promise<void>;

/**
 * 应用初始化：加载工作目录、写入初始日志、触发目录校验/进程检查。
 * 接受目录初始化函数以复用同一逻辑（用于手动选择与开机恢复）。
 */
export const useAppInit = (initializeDirectory: DirectoryInitializer) => {
  const {
    isInitialized,
    setInitialized,
    setAppLoading,
    setShowWelcomeModal,
    addLog,
  } = useAppStore();

  useEffect(() => {
    if (isInitialized) return;

    let cancelled = false;

    const run = async () => {
      setAppLoading(true);

      // 写入启动日志
      addLog('info', '🚀 Duck CLI GUI 已启动');

      try {
        const savedDirectory = await ConfigManager.getWorkingDirectory();

        if (cancelled) return;

        if (savedDirectory) {
          await initializeDirectory(savedDirectory);
        } else {
          setShowWelcomeModal(true);
        }
      } catch (error) {
        addLog('error', `初始化失败: ${error}`);
        setShowWelcomeModal(true);
      } finally {
        if (cancelled) return;
        setInitialized(true);
        setAppLoading(false);
      }
    };

    run();

    return () => {
      cancelled = true;
    };
  }, [isInitialized, addLog, initializeDirectory, setAppLoading, setInitialized, setShowWelcomeModal]);
};


