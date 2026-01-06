import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../store/appStore';

interface CliStreamEvent {
  command_id?: string;
  stream: 'stdout' | 'stderr';
  chunk: string;
  seq: number;
  timestamp?: string;
}

interface CliCompleteEvent {
  command_id?: string;
  exit_code: number;
  duration_ms?: number;
  timestamp?: string;
}

/**
 * 统一注册 CLI 事件监听，并在组件卸载时清理。
 */
export const useCliEvents = () => {
  const { addLog, setExecuting } = useAppStore();

  useEffect(() => {
    let unlistenOutput: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenComplete: (() => void) | undefined;

    const setup = async () => {
      try {
        unlistenOutput = await listen<CliStreamEvent | string>('cli-output', (event) => {
          if (typeof event.payload === 'string') {
            const output = event.payload.trim();
            if (output) addLog('info', output);
            return;
          }
          const payload = event.payload as CliStreamEvent;
          const chunk = (payload.chunk ?? '').trim();
          if (!chunk) return;
          addLog('info', chunk);
        });

        unlistenError = await listen<CliStreamEvent | string>('cli-error', (event) => {
          if (typeof event.payload === 'string') {
            const output = event.payload.trim();
            if (output) addLog('error', output);
            return;
          }
          const payload = event.payload as CliStreamEvent;
          const chunk = (payload.chunk ?? '').trim();
          if (!chunk) return;
          addLog('error', chunk);
        });

        unlistenComplete = await listen<CliCompleteEvent | number>('cli-complete', (event) => {
          if (typeof event.payload === 'number') {
            const exitCode = Number(event.payload ?? -1);
            setExecuting(false);
            addLog(exitCode === 0 ? 'success' : 'error', `命令执行${exitCode === 0 ? '完成' : '失败'} (退出码: ${exitCode})`);
            addLog('info', '─'.repeat(50));
            return;
          }

          const payload = event.payload as CliCompleteEvent;
          const exitCode = payload.exit_code ?? -1;
          setExecuting(false);

          if (exitCode === 0) {
            addLog('success', `命令执行完成 (退出码: ${exitCode})`);
          } else {
            addLog('error', `命令执行失败 (退出码: ${exitCode})`);
          }

          if (payload.duration_ms !== undefined) {
            addLog('info', `耗时: ${payload.duration_ms} ms`);
          }

          addLog('info', '─'.repeat(50));
        });
      } catch (error) {
        console.error('设置 CLI 事件监听失败', error);
      }
    };

    setup();

    return () => {
      unlistenOutput?.();
      unlistenError?.();
      unlistenComplete?.();
    };
  }, [addLog, setExecuting]);
};

