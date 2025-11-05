import React from 'react';
import { FolderIcon } from '@heroicons/react/24/outline';
import { useApp } from '../context/AppContext';

interface HeaderProps {
  onChangeDirectory?: () => void;
}

export const Header: React.FC<HeaderProps> = ({ onChangeDirectory }) => {
  const { workingDirectory, isDirectoryValid } = useApp();

  return (
    <header className="bg-white border-b border-gray-200 px-6 py-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center space-x-3">
          <h1 className="text-xl font-bold text-gray-900">🦆 Duck CLI</h1>
        </div>
        
        <div className="flex items-center space-x-3">
          <div className="flex items-center space-x-2">
            <FolderIcon className="w-5 h-5 text-gray-500" />
            <span className="text-sm text-gray-600">工作目录:</span>
            <span className={`text-sm font-medium ${isDirectoryValid ? 'text-gray-900' : 'text-red-600'}`}>
              {workingDirectory || '未设置'}
            </span>
          </div>
          
          {onChangeDirectory && (
            <button
              onClick={onChangeDirectory}
              className="px-3 py-1 text-sm text-blue-600 hover:text-blue-700 hover:bg-blue-50 rounded transition-colors"
            >
              更改
            </button>
          )}
        </div>
      </div>
    </header>
  );
};
