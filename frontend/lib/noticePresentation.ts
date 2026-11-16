import type { Notice } from './noticeGateway';
export const noticeNeedsInput = (notice: Notice) => !notice.finished && notice.actions.some(action => action.enabled && (action.kind === 'reply' || action.kind === 'permission'));
export const noticeIsActive = (notice: Notice) => !notice.finished;
export const noticeStatusLabel = (notice: Notice) => notice.status.label;
