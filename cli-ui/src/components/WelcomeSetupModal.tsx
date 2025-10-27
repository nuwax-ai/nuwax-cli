import React, { useState } from 'react';
import { 
  FolderIcon, 
  ExclamationTriangleIcon,
  CheckCircleIcon,
  XMarkIcon
} from '@heroicons/react/24/outline';
import { DialogManager, ConfigManager, FileSystemManager } from '../utils/tauri';

interface WelcomeSetupModalProps {
  isOpen: boolean;
  onComplete: (directory: string) => void;
  onSkip: () => void;
}

const WelcomeSetupModal: React.FC<WelcomeSetupModalProps> = ({ 
  isOpen, 
  onComplete, 
  onSkip 
}) => {
  const [selectedDirectory, setSelectedDirectory] = useState<string>('');
  const [isValidating, setIsValidating] = useState(false);
  const [validationResult, setValidationResult] = useState<{
    valid: boolean;
    error?: string;
  } | null>(null);

  // 选择目录
  const handleSelectDirectory = async () => {
    try {
      const directory = await DialogManager.selectDirectory();
      if (directory) {
        setSelectedDirectory(directory);
        await validateDirectory(directory);
      }
    } catch (error) {
      console.error('目录选择失败:', error);
    }
  };

  // 验证目录
  const validateDirectory = async (path: string) => {
    setIsValidating(true);
    setValidationResult(null);
    
    try {
      const result = await FileSystemManager.validateDirectory(path);
      setValidationResult(result);
    } catch (error) {
      setValidationResult({
        valid: false,
        error: `验证失败: ${error}`
      });
    } finally {
      setIsValidating(false);
    }
  };

  // 确认并开始
  const handleConfirm = async () => {
    if (!selectedDirectory || !validationResult?.valid) {
      return;
    }

    try {
      // 保存工作目录配置
      await ConfigManager.setWorkingDirectory(selectedDirectory);
      onComplete(selectedDirectory);
    } catch (error) {
      console.error('保存配置失败:', error);
      await DialogManager.showMessage('错误', '保存配置失败', 'error');
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-gray-600 bg-opacity-50 overflow-y-auto h-full w-full z-50">
      <div className="relative top-20 mx-auto p-5 border w-11/12 max-w-2xl shadow-lg rounded-md bg-white">
        {/* 头部 */}
        <div className="flex items-center justify-between pb-4 border-b">
          <div className="flex items-center space-x-3">
            <div className="text-4xl">🦆</div>
            <div>
              <h3 className="text-lg font-semibold text-gray-900">
                欢迎使用 Duck CLI GUI
              </h3>
              <p className="text-sm text-gray-600">
                开始前，请选择一个工作目录
              </p>
            </div>
          </div>
        </div>

        {/* 内容区域 */}
        <div className="mt-6">
          {/* 说明信息 */}
          <div className="bg-blue-50 border border-blue-200 rounded-md p-4 mb-6">
            <div className="flex">
              <div className="flex-shrink-0">
                <ExclamationTriangleIcon className="h-5 w-5 text-blue-400" />
              </div>
              <div className="ml-3">
                <h4 className="text-sm font-medium text-blue-800">
                  关于工作目录
                </h4>
                <div className="mt-2 text-sm text-blue-700">
                  <ul className="list-disc list-inside space-y-1">
                    <li>工作目录是 Duck CLI 执行所有命令的基础路径</li>
                    <li>建议选择一个空目录或新建目录</li>
                    <li>确保目录有读写权限</li>
                    <li>避免选择系统关键目录（如 /、/usr、/System 等）</li>
                  </ul>
                </div>
              </div>
            </div>
          </div>

          {/* 目录选择区域 */}
          <div className="space-y-4">
            <label className="block text-sm font-medium text-gray-700">
              选择工作目录
            </label>
            
            <div className="flex space-x-3">
              <div className="flex-1">
                <input
                  type="text"
                  value={selectedDirectory}
                  onChange={(e) => setSelectedDirectory(e.target.value)}
                  placeholder="请选择或输入工作目录路径..."
                  className="w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500"
                />
              </div>
              <button
                onClick={handleSelectDirectory}
                className="inline-flex items-center px-4 py-2 border border-gray-300 shadow-sm text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
              >
                <FolderIcon className="h-4 w-4 mr-2" />
                浏览...
              </button>
            </div>

            {/* 验证状态 */}
            {selectedDirectory && (
              <div className="mt-3">
                {isValidating ? (
                  <div className="flex items-center space-x-2 text-sm text-blue-600">
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-blue-600"></div>
                    <span>正在验证目录...</span>
                  </div>
                ) : validationResult ? (
                  <div className={`flex items-center space-x-2 text-sm ${
                    validationResult.valid ? 'text-green-600' : 'text-red-600'
                  }`}>
                    {validationResult.valid ? (
                      <CheckCircleIcon className="h-4 w-4" />
                    ) : (
                      <XMarkIcon className="h-4 w-4" />
                    )}
                    <span>
                      {validationResult.valid 
                        ? '目录验证通过，可以使用' 
                        : `验证失败: ${validationResult.error}`
                      }
                    </span>
                  </div>
                ) : null}
              </div>
            )}
          </div>

          {/* 建议目录 */}
          <div className="mt-6">
            <h4 className="text-sm font-medium text-gray-700 mb-3">
              推荐目录示例
            </h4>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              {[
                { path: '~/Documents/duck-projects', desc: '文档目录下' },
                { path: '~/Desktop/duck-workspace', desc: '桌面工作区' },
                { path: '/Users/[用户名]/duck-cli', desc: '用户目录下' },
                { path: '~/Development/duck', desc: '开发目录下' }
              ].map((suggestion, index) => (
                <button
                  key={index}
                  onClick={() => {
                    setSelectedDirectory(suggestion.path);
                    validateDirectory(suggestion.path);
                  }}
                  className="text-left p-3 border border-gray-200 rounded-md hover:bg-gray-50 hover:border-gray-300 transition-colors"
                >
                  <div className="text-sm font-medium text-gray-900">
                    {suggestion.path}
                  </div>
                  <div className="text-xs text-gray-500">
                    {suggestion.desc}
                  </div>
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* 底部按钮 */}
        <div className="mt-8 flex justify-between">
          <button
            onClick={onSkip}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
          >
            稍后设置
          </button>
          
          <button
            onClick={handleConfirm}
            disabled={!selectedDirectory || !validationResult?.valid}
            className="px-6 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            确认并开始使用
          </button>
        </div>
      </div>
    </div>
  );
};

export default WelcomeSetupModal; 