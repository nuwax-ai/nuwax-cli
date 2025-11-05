import React from 'react';
import { Outlet } from 'react-router-dom';
import { Header } from './Header';
import { Sidebar } from './Sidebar';

interface MainLayoutProps {
  onChangeDirectory?: () => void;
}

export const MainLayout: React.FC<MainLayoutProps> = ({ onChangeDirectory }) => {
  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <Header onChangeDirectory={onChangeDirectory} />
      
      {/* Main Content Area */}
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <Sidebar />
        
        {/* Page Content */}
        <main className="flex-1 overflow-auto bg-gray-100">
          <Outlet />
        </main>
      </div>
    </div>
  );
};
