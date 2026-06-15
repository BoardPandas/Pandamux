import { useState } from 'react';
import SidebarSettings from './SidebarSettings';
import WorkspaceSettings from './WorkspaceSettings';
import TerminalSettings from './TerminalSettings';
import NotificationSettings from './NotificationSettings';
import BrowserSettings from './BrowserSettings';
import KeyboardSettings from './KeyboardSettings';
import QuickLaunchSettings from './QuickLaunchSettings';
import '../../styles/settings.css';

const TABS = ['Sidebar', 'Workspace', 'Terminal', 'Notifications', 'Browser', 'Profiles', 'Shortcuts'] as const;

interface SettingsWindowProps {
  onClose: () => void;
}

export default function SettingsWindow({ onClose }: SettingsWindowProps) {
  const [activeTab, setActiveTab] = useState<typeof TABS[number]>('Terminal');

  return (
    <div
      className="settings-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="settings-window">
        <div className="settings-header">
          <h2>Settings</h2>
          <button className="settings-close" onClick={onClose}>
            ×
          </button>
        </div>
        <div className="settings-body">
          <div className="settings-tabs">
            {TABS.map((tab) => (
              <button
                key={tab}
                className={`settings-tab ${activeTab === tab ? 'settings-tab--active' : ''}`}
                onClick={() => setActiveTab(tab)}
              >
                {tab}
              </button>
            ))}
          </div>
          <div className="settings-content">
            {activeTab === 'Sidebar' && <SidebarSettings />}
            {activeTab === 'Workspace' && <WorkspaceSettings />}
            {activeTab === 'Terminal' && <TerminalSettings />}
            {activeTab === 'Notifications' && <NotificationSettings />}
            {activeTab === 'Browser' && <BrowserSettings />}
            {activeTab === 'Profiles' && <QuickLaunchSettings />}
            {activeTab === 'Shortcuts' && <KeyboardSettings />}
          </div>
        </div>
      </div>
    </div>
  );
}
