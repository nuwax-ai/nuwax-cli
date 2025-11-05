import React, { useState } from 'react';
import { NavLink } from 'react-router-dom';
import {
  ChartBarIcon,
  RocketLaunchIcon,
  CubeIcon,
  CircleStackIcon,
  DocumentTextIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
} from '@heroicons/react/24/outline';

interface NavItem {
  path: string;
  name: string;
  icon: React.ComponentType<{ className?: string }>;
}

const navItems: NavItem[] = [
  { path: '/overview', name: '概览', icon: ChartBarIcon },
  { path: '/deploy', name: '部署', icon: RocketLaunchIcon },
  { path: '/containers', name: '容器', icon: CubeIcon },
  { path: '/backup', name: '备份', icon: CircleStackIcon },
  { path: '/logs', name: '日志', icon: DocumentTextIcon },
];

export const Sidebar: React.FC = () => {
  const [isCollapsed, setIsCollapsed] = useState(false);

  return (
    <aside
      className={`bg-gray-50 border-r border-gray-200 transition-all duration-300 ${
        isCollapsed ? 'w-16' : 'w-64'
      }`}
    >
      <nav className="flex flex-col h-full">
        {/* Navigation Items */}
        <div className="flex-1 py-4">
          {navItems.map((item) => (
            <NavLink
              key={item.path}
              to={item.path}
              className={({ isActive }) =>
                `flex items-center px-4 py-3 text-sm font-medium transition-colors ${
                  isActive
                    ? 'bg-blue-50 text-blue-700 border-r-2 border-blue-700'
                    : 'text-gray-700 hover:bg-gray-100 hover:text-gray-900'
                }`
              }
            >
              {({ isActive }) => (
                <>
                  <item.icon
                    className={`${isCollapsed ? 'w-6 h-6' : 'w-5 h-5'} ${
                      isActive ? 'text-blue-700' : 'text-gray-500'
                    }`}
                  />
                  {!isCollapsed && <span className="ml-3">{item.name}</span>}
                </>
              )}
            </NavLink>
          ))}
        </div>

        {/* Collapse Toggle */}
        <div className="border-t border-gray-200 p-4">
          <button
            onClick={() => setIsCollapsed(!isCollapsed)}
            className="w-full flex items-center justify-center px-3 py-2 text-sm text-gray-600 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors"
            title={isCollapsed ? '展开侧边栏' : '折叠侧边栏'}
          >
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 h-5" />
            ) : (
              <>
                <ChevronLeftIcon className="w-5 h-5" />
                <span className="ml-2">折叠</span>
              </>
            )}
          </button>
        </div>
      </nav>
    </aside>
  );
};
