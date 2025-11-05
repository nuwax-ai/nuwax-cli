// 日志条目类型
export interface LogEntry {
  id: string;
  timestamp: string;
  type: 'info' | 'success' | 'error' | 'warning' | 'command';
  message: string;
  command?: string;
  args?: string[];
}

// 日志管理配置
export interface LogConfig {
  maxEntries: number;        // 最大日志条目数
  trimBatchSize: number;     // 一次清理的数量
}

// 默认日志配置
export const DEFAULT_LOG_CONFIG: LogConfig = {
  maxEntries: 100000,        // 最多保留100000条日志
  trimBatchSize: 10000,      // 超出时一次清理10000条（保留90000条）
};

// 工作目录状态
export interface WorkingDirectoryState {
  path: string | null;
  isValid: boolean;
  isChecking: boolean;
  error?: string;
}

// 应用状态
export interface AppState {
  workingDirectory: WorkingDirectoryState;
  logs: LogEntry[];
  isLoading: boolean;
  logConfig: LogConfig;
}

// 命令执行结果
export interface CommandResult {
  success: boolean;
  output: string;
  error?: string;
}

// 参数类型定义
export interface CommandParameter {
  name: string;           // 参数名称
  label: string;          // 显示标签
  type: 'text' | 'number' | 'boolean' | 'select' | 'multiselect';
  required?: boolean;     // 是否必填
  defaultValue?: any;     // 默认值
  placeholder?: string;   // 占位符
  description?: string;   // 参数说明
  options?: Array<{       // 选择类型的选项
    value: string;
    label: string;
  }>;
  min?: number;          // 数字类型的最小值
  max?: number;          // 数字类型的最大值
  validation?: {         // 验证规则
    pattern?: string;
    message?: string;
  };
}

// 命令配置定义
export interface CommandConfig {
  id: string;
  name: string;
  description: string;
  parameters: CommandParameter[];
  examples?: string[];    // 使用示例
}

// 参数输入结果
export interface ParameterInputResult {
  [key: string]: any;
}

// 参数输入模态框属性
export interface ParameterInputModalProps {
  isOpen: boolean;
  commandConfig: CommandConfig | null;
  onConfirm: (parameters: ParameterInputResult) => void;
  onCancel: () => void;
}

// 容器信息类型
export interface ContainerInfo {
  name: string;
  status: 'running' | 'stopped' | 'starting' | 'stopping' | 'restarting' | 'paused' | 'exited' | 'dead' | 'unknown';
  image: string;
  ports: string[];
  id?: string;
  created?: string;
  uptime?: string;
}

// 备份记录类型
export interface BackupRecord {
  id: number;
  backup_type: 'Manual' | 'PreUpgrade';
  created_at: string;
  service_version: string;
  file_path: string;
  file_size?: number;
  file_exists: boolean;
}

// 工作目录验证结果
export interface DirectoryValidation {
  valid: boolean;
  message: string;
  has_config: boolean;
  has_docker_compose: boolean;
}

// 页面路由类型
export interface PageRoute {
  path: string;
  name: string;
  icon: React.ComponentType<{ className?: string }>;
  component: React.ComponentType;
}

// 容器状态统计
export interface ContainerStats {
  total: number;
  running: number;
  stopped: number;
  other: number;
} 