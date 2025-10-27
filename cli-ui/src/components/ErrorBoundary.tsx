import { Component, ErrorInfo, ReactNode } from 'react';
import { ExclamationTriangleIcon, ArrowPathIcon } from '@heroicons/react/24/outline';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error?: Error;
  errorInfo?: ErrorInfo;
}

class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): State {
    // 更新状态以显示错误页面
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    // 记录错误信息
    console.error('ErrorBoundary捕获到错误:', error, errorInfo);
    
    this.setState({
      error,
      errorInfo
    });

    // 这里可以将错误信息发送到日志服务
    this.logError(error, errorInfo);
  }

  private logError = (error: Error, errorInfo: ErrorInfo) => {
    // 构建错误报告
    const errorReport = {
      message: error.message,
      stack: error.stack,
      componentStack: errorInfo.componentStack,
      timestamp: new Date().toISOString(),
      userAgent: navigator.userAgent,
      url: window.location.href
    };

    // 存储到本地存储以便调试
    try {
      const existingErrors = JSON.parse(localStorage.getItem('duck-cli-errors') || '[]');
      existingErrors.push(errorReport);
      
      // 只保留最近的50个错误
      if (existingErrors.length > 50) {
        existingErrors.splice(0, existingErrors.length - 50);
      }
      
      localStorage.setItem('duck-cli-errors', JSON.stringify(existingErrors));
    } catch (storageError) {
      console.error('保存错误信息失败:', storageError);
    }
  };

  private handleReload = () => {
    // 清除错误状态并重新加载
    this.setState({ hasError: false, error: undefined, errorInfo: undefined });
    window.location.reload();
  };

  private handleReset = () => {
    // 只重置错误状态，不重新加载页面
    this.setState({ hasError: false, error: undefined, errorInfo: undefined });
  };

  private copyErrorToClipboard = async () => {
    const { error, errorInfo } = this.state;
    
    const errorText = `
Duck CLI GUI 错误报告
时间: ${new Date().toLocaleString()}
版本: 0.1.0

错误信息:
${error?.message || '未知错误'}

错误堆栈:
${error?.stack || '无堆栈信息'}

组件堆栈:
${errorInfo?.componentStack || '无组件堆栈'}

用户代理:
${navigator.userAgent}

URL:
${window.location.href}
    `.trim();

    try {
      await navigator.clipboard.writeText(errorText);
      alert('错误信息已复制到剪贴板');
    } catch (clipboardError) {
      console.error('复制到剪贴板失败:', clipboardError);
      
      // 降级：创建一个临时文本区域
      const textarea = document.createElement('textarea');
      textarea.value = errorText;
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand('copy');
      document.body.removeChild(textarea);
      alert('错误信息已复制到剪贴板');
    }
  };

  render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen bg-gray-50 flex flex-col justify-center py-12 sm:px-6 lg:px-8">
          <div className="sm:mx-auto sm:w-full sm:max-w-md">
            <div className="mx-auto h-24 w-24 text-red-500">
              <ExclamationTriangleIcon className="h-full w-full" />
            </div>
            <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
              应用发生错误
            </h2>
            <p className="mt-2 text-center text-sm text-gray-600">
              Duck CLI GUI 遇到了一个意外错误
            </p>
          </div>

          <div className="mt-8 sm:mx-auto sm:w-full sm:max-w-md">
            <div className="bg-white py-8 px-4 shadow sm:rounded-lg sm:px-10">
              <div className="space-y-6">
                {/* 错误信息 */}
                <div className="bg-red-50 border border-red-200 rounded-md p-4">
                  <div className="flex">
                    <ExclamationTriangleIcon className="h-5 w-5 text-red-400 flex-shrink-0" />
                    <div className="ml-3">
                      <h3 className="text-sm font-medium text-red-800">
                        错误详情
                      </h3>
                      <div className="mt-2 text-sm text-red-700">
                        <p className="font-mono text-xs break-all">
                          {this.state.error?.message || '未知错误'}
                        </p>
                      </div>
                    </div>
                  </div>
                </div>

                {/* 建议操作 */}
                <div className="bg-blue-50 border border-blue-200 rounded-md p-4">
                  <h4 className="text-sm font-medium text-blue-800 mb-2">
                    建议操作
                  </h4>
                  <ul className="text-sm text-blue-700 space-y-1">
                    <li>• 尝试重新加载应用</li>
                    <li>• 检查工作目录是否有效</li>
                    <li>• 确保有足够的磁盘空间</li>
                    <li>• 检查网络连接</li>
                    <li>• 如果问题持续，请联系技术支持</li>
                  </ul>
                </div>

                {/* 操作按钮 */}
                <div className="space-y-3">
                  <button
                    onClick={this.handleReset}
                    className="w-full flex justify-center items-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
                  >
                    <ArrowPathIcon className="h-4 w-4 mr-2" />
                    尝试恢复
                  </button>

                  <button
                    onClick={this.handleReload}
                    className="w-full flex justify-center items-center py-2 px-4 border border-gray-300 rounded-md shadow-sm text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
                  >
                    重新加载应用
                  </button>

                  <button
                    onClick={this.copyErrorToClipboard}
                    className="w-full flex justify-center items-center py-2 px-4 border border-gray-300 rounded-md shadow-sm text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
                  >
                    复制错误信息
                  </button>
                </div>

                {/* 开发模式下的详细错误信息 */}
                {import.meta.env.DEV && (
                  <details className="mt-6">
                    <summary className="text-sm font-medium text-gray-900 cursor-pointer">
                      开发信息（仅开发环境可见）
                    </summary>
                    <div className="mt-2 text-xs font-mono text-gray-600 bg-gray-100 p-3 rounded overflow-auto max-h-40">
                      <div className="mb-2">
                        <strong>错误堆栈:</strong>
                        <pre className="whitespace-pre-wrap">
                          {this.state.error?.stack}
                        </pre>
                      </div>
                      <div>
                        <strong>组件堆栈:</strong>
                        <pre className="whitespace-pre-wrap">
                          {this.state.errorInfo?.componentStack}
                        </pre>
                      </div>
                    </div>
                  </details>
                )}
              </div>
            </div>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

export default ErrorBoundary; 