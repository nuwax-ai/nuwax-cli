import { DuckCliManager } from '../utils/tauri';

export interface ExecuteOptions {
  commandId?: string;
  workingDirectory?: string;
  timeoutMs?: number;
}

export interface ExecuteResult {
  success: boolean;
  exitCode: number;
  stdout: string;
  stderr: string;
  durationMs: number;
}

const withTimeout = <T>(promise: Promise<T>, timeoutMs?: number, timeoutMessage = '命令执行超时'): Promise<T> => {
  if (!timeoutMs || timeoutMs <= 0) return promise;

  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      setTimeout(() => reject(new Error(timeoutMessage)), timeoutMs);
    }),
  ]);
};

export const cliGateway = {
  /**
   * 统一执行 duck-cli 命令，默认走后端智能策略（sidecar -> system）。
   */
  async execute(args: string[], options: ExecuteOptions = {}): Promise<ExecuteResult> {
    const start = performance.now ? performance.now() : Date.now();
    const run = async () => {
      const result = await DuckCliManager.executeSmart(args, options.workingDirectory, options.commandId);
      return {
        success: result.success,
        exitCode: result.exit_code,
        stdout: result.stdout,
        stderr: result.stderr,
      };
    };

    const { success, exitCode, stdout, stderr } = await withTimeout(run(), options.timeoutMs);
    const durationMs = (performance.now ? performance.now() : Date.now()) - start;

    return {
      success,
      exitCode,
      stdout,
      stderr,
      durationMs,
    };
  },
};

