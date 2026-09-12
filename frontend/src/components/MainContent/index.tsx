'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { translateUI } from '@/i18n';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed, toggleCollapse } = useSidebar();
  return (
    <main
      className={`v2-main h-screen min-w-0 flex-1 overflow-hidden border-l border-border/70 bg-background transition-[margin] duration-200 ease-out ${
        isCollapsed ? 'ml-16' : 'ml-[232px]'
      }`}
    >
      <div className="v2-route">{children}</div>
      {!isCollapsed && <button type="button" className="v2-mobile-dismiss" aria-label={translateUI('Collapse sidebar')} onClick={toggleCollapse} />}
    </main>
  );
};

export default MainContent;
