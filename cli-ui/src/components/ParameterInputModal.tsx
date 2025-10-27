import React, { useState, useEffect } from 'react';
import { 
  XMarkIcon, 
  QuestionMarkCircleIcon,
  CheckIcon,
  XCircleIcon
} from '@heroicons/react/24/outline';
import { CommandParameter, ParameterInputResult, ParameterInputModalProps } from '../types';

const ParameterInputModal: React.FC<ParameterInputModalProps> = ({
  isOpen,
  commandConfig,
  onConfirm,
  onCancel
}) => {
  const [parameters, setParameters] = useState<ParameterInputResult>({});
  const [errors, setErrors] = useState<{ [key: string]: string }>({});

  // 初始化参数默认值
  useEffect(() => {
    if (commandConfig) {
      const defaultValues: ParameterInputResult = {};
      commandConfig.parameters.forEach(param => {
        if (param.defaultValue !== undefined && param.defaultValue !== null) {
          defaultValues[param.name] = param.defaultValue;
        }
      });
      setParameters(defaultValues);
      setErrors({});
    }
  }, [commandConfig]);

  // 更新参数值
  const updateParameter = (name: string, value: any) => {
    setParameters(prev => ({
      ...prev,
      [name]: value
    }));
    
    // 清除该参数的错误
    if (errors[name]) {
      setErrors(prev => {
        const newErrors = { ...prev };
        delete newErrors[name];
        return newErrors;
      });
    }
  };

  // 验证参数
  const validateParameters = (): boolean => {
    if (!commandConfig) return false;

    const newErrors: { [key: string]: string } = {};
    
    commandConfig.parameters.forEach(param => {
      const value = parameters[param.name];
      
      // 必填验证
      if (param.required && (value === undefined || value === null || value === '')) {
        newErrors[param.name] = `${param.label} 是必填项`;
        return;
      }
      
      // 数字范围验证
      if (param.type === 'number' && value !== undefined && value !== '') {
        const numValue = Number(value);
        if (isNaN(numValue)) {
          newErrors[param.name] = `${param.label} 必须是数字`;
          return;
        }
        if (param.min !== undefined && numValue < param.min) {
          newErrors[param.name] = `${param.label} 最小值为 ${param.min}`;
          return;
        }
        if (param.max !== undefined && numValue > param.max) {
          newErrors[param.name] = `${param.label} 最大值为 ${param.max}`;
          return;
        }
      }
      
      // 正则验证
      if (param.validation && param.validation.pattern && value) {
        const pattern = new RegExp(param.validation.pattern);
        if (!pattern.test(value)) {
          newErrors[param.name] = param.validation.message || `${param.label} 格式不正确`;
        }
      }
    });

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  // 确认执行
  const handleConfirm = () => {
    if (validateParameters()) {
      onConfirm(parameters);
    }
  };

  // 渲染参数输入控件
  const renderParameterInput = (param: CommandParameter) => {
    const value = parameters[param.name];
    const hasError = errors[param.name];

    const inputBaseClass = `mt-1 block w-full rounded-md shadow-sm focus:ring-blue-500 focus:border-blue-500 sm:text-sm ${
      hasError ? 'border-red-300' : 'border-gray-300'
    }`;

    switch (param.type) {
      case 'text':
        return (
          <input
            type="text"
            value={value || ''}
            onChange={(e) => updateParameter(param.name, e.target.value)}
            placeholder={param.placeholder}
            className={inputBaseClass}
          />
        );

      case 'number':
        return (
          <input
            type="number"
            value={value || ''}
            onChange={(e) => updateParameter(param.name, e.target.value)}
            placeholder={param.placeholder}
            min={param.min}
            max={param.max}
            className={inputBaseClass}
          />
        );

      case 'boolean':
        return (
          <div className="flex items-center">
            <input
              type="checkbox"
              checked={value || false}
              onChange={(e) => updateParameter(param.name, e.target.checked)}
              className="h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded"
            />
            <span className="ml-2 text-sm text-gray-600">{param.description}</span>
          </div>
        );

      case 'select':
        // 获取当前值，优先使用参数值，其次使用默认值
        const currentValue = value !== undefined ? value : (param.defaultValue || '');
        
        return (
          <select
            value={currentValue}
            onChange={(e) => updateParameter(param.name, e.target.value)}
            className={inputBaseClass}
          >
            {!param.required && <option value="">请选择...</option>}
            {param.options?.map(option => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        );

      case 'multiselect':
        return (
          <div className="space-y-2">
            {param.options?.map(option => (
              <div key={option.value} className="flex items-center">
                <input
                  type="checkbox"
                  checked={(value || []).includes(option.value)}
                  onChange={(e) => {
                    const currentValues = value || [];
                    const newValues = e.target.checked
                      ? [...currentValues, option.value]
                      : currentValues.filter((v: string) => v !== option.value);
                    updateParameter(param.name, newValues);
                  }}
                  className="h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded"
                />
                <span className="ml-2 text-sm text-gray-700">{option.label}</span>
              </div>
            ))}
          </div>
        );

      default:
        return null;
    }
  };

  if (!isOpen || !commandConfig) return null;

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto">
      <div className="flex items-center justify-center min-h-screen pt-4 px-4 pb-20 text-center sm:block sm:p-0">
        {/* 背景遮罩 */}
        <div className="fixed inset-0 bg-gray-500 bg-opacity-75 transition-opacity" onClick={onCancel} />

        {/* 模态框 */}
        <div className="inline-block align-bottom bg-white rounded-lg px-4 pt-5 pb-4 text-left overflow-hidden shadow-xl transform transition-all sm:my-8 sm:align-middle sm:max-w-2xl sm:w-full sm:p-6">
          
          {/* 标题栏 */}
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-lg font-medium text-gray-900">
              {commandConfig.name} - 参数设置
            </h3>
            <button
              onClick={onCancel}
              className="text-gray-400 hover:text-gray-500"
            >
              <XMarkIcon className="h-6 w-6" />
            </button>
          </div>

          {/* 命令描述 */}
          <div className="mb-6">
            <p className="text-sm text-gray-600">{commandConfig.description}</p>
          </div>

          {/* 参数输入表单 */}
          <div className="space-y-6">
            {commandConfig.parameters.map(param => (
              <div key={param.name}>
                <label className="block text-sm font-medium text-gray-700">
                  {param.label}
                  {param.required && <span className="text-red-500 ml-1">*</span>}
                  {param.description && param.type !== 'boolean' && (
                    <div className="flex items-center mt-1">
                      <QuestionMarkCircleIcon className="h-4 w-4 text-gray-400 mr-1" />
                      <span className="text-xs text-gray-500">{param.description}</span>
                    </div>
                  )}
                </label>
                
                {renderParameterInput(param)}
                
                {/* 错误提示 */}
                {errors[param.name] && (
                  <div className="mt-1 flex items-center">
                    <XCircleIcon className="h-4 w-4 text-red-500 mr-1" />
                    <span className="text-sm text-red-600">{errors[param.name]}</span>
                  </div>
                )}
              </div>
            ))}
          </div>

          {/* 使用示例 */}
          {commandConfig.examples && commandConfig.examples.length > 0 && (
            <div className="mt-6 p-4 bg-gray-50 rounded-md">
              <h4 className="text-sm font-medium text-gray-700 mb-2">使用示例:</h4>
              <div className="space-y-1">
                {commandConfig.examples.map((example, index) => (
                  <code key={index} className="block text-xs text-gray-600 font-mono">
                    {example}
                  </code>
                ))}
              </div>
            </div>
          )}

          {/* 底部按钮 */}
          <div className="mt-6 flex justify-end space-x-3">
            <button
              onClick={onCancel}
              className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
            >
              取消
            </button>
            <button
              onClick={handleConfirm}
              className="px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
            >
              <CheckIcon className="h-4 w-4 mr-1 inline" />
              确认执行
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ParameterInputModal; 