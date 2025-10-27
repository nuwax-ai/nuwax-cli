import { CommandConfig } from '../types';

// 命令配置定义
export const commandConfigs: { [key: string]: CommandConfig } = {
  // 一键部署
  'auto-upgrade-deploy': {
    id: 'auto-upgrade-deploy',
    name: '一键部署',
    description: '自动升级并部署Docker服务，支持指定前端服务端口',
    parameters: [
      {
        name: 'port',
        label: '前端服务端口',
        type: 'number',
        required: false,
        defaultValue: 80,
        min: 1,
        max: 65535,
        placeholder: '80',
        description: '指定frontend服务的端口号，对应docker-compose.yml中的FRONTEND_HOST_PORT变量'
      }
    ],
    examples: [
      'duck-cli auto-upgrade-deploy run',
      'duck-cli auto-upgrade-deploy run --port 8080',
      'duck-cli auto-upgrade-deploy run --port 3000'
    ]
  },

  // 延迟部署
  'delay-deploy': {
    id: 'delay-deploy',
    name: '延迟部署',
    description: '延迟指定时间后执行自动升级部署',
    parameters: [
      {
        name: 'time',
        label: '延迟时间',
        type: 'number',
        required: true,
        min: 1,
        max: 9999,
        placeholder: '2',
        description: '延迟时间数值'
      },
      {
        name: 'unit',
        label: '时间单位',
        type: 'select',
        required: true,
        defaultValue: 'hours',
        options: [
          { value: 'minutes', label: '分钟' },
          { value: 'hours', label: '小时' },
          { value: 'days', label: '天' }
        ],
        description: '时间单位'
      }
    ],
    examples: [
      'duck-cli auto-upgrade-deploy delay-time-deploy 2 --unit hours',
      'duck-cli auto-upgrade-deploy delay-time-deploy 30 --unit minutes',
      'duck-cli auto-upgrade-deploy delay-time-deploy 1 --unit days'
    ]
  },

  // 检查更新
  'check-update': {
    id: 'check-update',
    name: '客户端检查更新',
    description: '检查客户端更新,或安装最新版本',
    parameters: [
      {
        name: 'action',
        label: '操作类型',
        type: 'select',
        required: true,
        defaultValue: 'check',
        options: [
          { value: 'check', label: '检查更新' },
          { value: 'install', label: '安装版本' }
        ],
        description: '选择执行的操作'
      },
      {
        name: 'version',
        label: '版本号',
        type: 'text',
        required: false,
        placeholder: '0.0.1',
        description: '指定要安装的版本号（留空则安装最新版本）'
      },
      {
        name: 'force',
        label: '强制重新安装',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '即使当前已是最新版本也强制重新安装'
      }
    ],
    examples: [
      'duck-cli check-update check',
      'duck-cli check-update install',
      'duck-cli check-update install --version 1.0.0',
      'duck-cli check-update install --force'
    ]
  },

  // 升级服务
  'upgrade': {
    id: 'upgrade',
    name: 'Docker服务升级',
    description: '下载Docker服务文件，支持全量下载和强制重新下载',
    parameters: [
      {
        name: 'full',
        label: '全量下载',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '下载完整的服务包'
      },
      {
        name: 'force',
        label: '强制重新下载',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '用于文件损坏时强制重新下载'
      },
      {
        name: 'check',
        label: '仅检查版本',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '只检查是否有可用的升级版本，不执行下载'
      }
    ],
    examples: [
      'duck-cli upgrade',
      'duck-cli upgrade --full',
      'duck-cli upgrade --force',
      'duck-cli upgrade --check'
    ]
  },

  // 初始化
  'init': {
    id: 'init',
    name: '初始化',
    description: '首次使用时初始化客户端，创建配置文件和数据库',
    parameters: [
      {
        name: 'force',
        label: '强制覆盖',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '如果配置文件已存在，强制覆盖'
      }
    ],
    examples: [
      'duck-cli init',
      'duck-cli init --force'
    ]
  },

  // 回滚服务
  'rollback': {
    id: 'rollback',
    name: '回滚服务',
    description: '从指定备份恢复服务',
    parameters: [
      {
        name: 'backup_id',
        label: '备份ID',
        type: 'number',
        required: true,
        min: 1,
        placeholder: '1',
        description: '要恢复的备份ID'
      },
      {
        name: 'force',
        label: '强制覆盖',
        type: 'boolean',
        required: false,
        defaultValue: false,
        description: '强制覆盖现有文件'
      }
    ],
    examples: [
      'duck-cli rollback 1',
      'duck-cli rollback 1 --force'
    ]
  },

  // 重启容器
  'restart-container': {
    id: 'restart-container',
    name: '重启容器',
    description: '重启指定的Docker容器',
    parameters: [
      {
        name: 'container_name',
        label: '容器名称',
        type: 'text',
        required: true,
        placeholder: 'frontend',
        description: '要重启的容器名称'
      }
    ],
    examples: [
      'duck-cli docker-service restart-container frontend',
      'duck-cli docker-service restart-container backend',
      'duck-cli docker-service restart-container database'
    ]
  },

  // 解压服务包
  'extract': {
    id: 'extract',
    name: '解压服务包',
    description: '解压Docker服务包到指定位置',
    parameters: [
      {
        name: 'file',
        label: '服务包文件',
        type: 'text',
        required: false,
        placeholder: '/path/to/docker.zip',
        description: '指定docker.zip文件路径（可选，默认使用当前版本的下载文件）'
      },
      {
        name: 'version',
        label: '目标版本',
        type: 'text',
        required: false,
        placeholder: '1.0.0',
        description: '目标版本（可选，默认使用当前配置版本）'
      }
    ],
    examples: [
      'duck-cli docker-service extract',
      'duck-cli docker-service extract --file /path/to/docker.zip',
      'duck-cli docker-service extract --version 1.0.0'
    ]
  },

  // 清理缓存
  'clean-downloads': {
    id: 'clean-downloads',
    name: '清理下载缓存',
    description: '清理下载缓存，保留最新的几个版本',
    parameters: [
      {
        name: 'keep',
        label: '保留版本数',
        type: 'number',
        required: false,
        defaultValue: 3,
        min: 1,
        max: 10,
        placeholder: '3',
        description: '保留的版本数量'
      }
    ],
    examples: [
      'duck-cli cache clean-downloads',
      'duck-cli cache clean-downloads --keep 5'
    ]
  },

  // Ducker 命令
  'ducker': {
    id: 'ducker',
    name: 'Ducker',
    description: '使用Ducker管理Docker容器',
    parameters: [
      {
        name: 'args',
        label: '命令参数',
        type: 'text',
        required: false,
        placeholder: '--help',
        description: '传递给ducker的参数'
      }
    ],
    examples: [
      'duck-cli ducker',
      'duck-cli ducker --help',
      'duck-cli ducker ps',
      'duck-cli ducker logs frontend'
    ]
  },

  // 自动备份
  'auto-backup': {
    id: 'auto-backup',
    name: '自动备份',
    description: '自动停止服务、创建备份、然后重启服务的完整流程',
    parameters: [], // 目前AutoBackup Run命令不需要额外参数
    examples: [
      'duck-cli auto-backup run',
      'duck-cli auto-backup status'
    ]
  }
};

// 获取命令配置
export const getCommandConfig = (commandId: string): CommandConfig | null => {
  return commandConfigs[commandId] || null;
};

// 检查命令是否需要参数输入
export const needsParameterInput = (commandId: string): boolean => {
  const config = getCommandConfig(commandId);
  return config !== null && config.parameters.length > 0;
}; 