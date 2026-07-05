import { create } from 'zustand';
import { WorkspaceSlice, createWorkspaceSlice } from './workspace-slice';
import { SettingsSlice, createSettingsSlice } from './settings-slice';
import { NotificationSlice, createNotificationSlice } from './notification-slice';
import { SurfaceSlice, createSurfaceSlice } from './surface-slice';
import { AgentSlice, createAgentSlice } from './agent-slice';
import { OrchestrationSlice, createOrchestrationSlice } from './orchestration-slice';

export type PandaMUXStore = WorkspaceSlice & SettingsSlice & NotificationSlice & SurfaceSlice & AgentSlice & OrchestrationSlice;

export const useStore = create<PandaMUXStore>()((...args) => ({
  ...createWorkspaceSlice(...args),
  ...createSettingsSlice(...args),
  ...createNotificationSlice(...args),
  ...createSurfaceSlice(...args),
  ...createAgentSlice(...args),
  ...createOrchestrationSlice(...args),
}));
