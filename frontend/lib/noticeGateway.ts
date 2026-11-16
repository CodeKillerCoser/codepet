import { codepetGateway } from './codepetGateway';
export type { Notice, NoticeSource, NoticeAction, NoticeActionInput } from '../../sdk/typescript/codepet-core-sdk/src/generated';
export const noticeList = (offset = 0, limit = 200) => codepetGateway('notice.list', { offset, limit });
export const noticeSourceSetEnabled = (pluginId: string, enabled: boolean) => codepetGateway('notice.source.setEnabled', { pluginId, enabled });
export const noticeActionExecute = (noticeId: string, actionId: string, expectedRevision: number, executionId: string, input: import('../../sdk/typescript/codepet-core-sdk/src/generated').NoticeActionInput) => codepetGateway('notice.action.execute', { noticeId, actionId, expectedRevision, executionId, input });
