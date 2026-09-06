import { invoke } from '@tauri-apps/api/core';
import { PROTOCOL_VERSION, type ProtocolMethod, type ProtocolRequestMap, type ProtocolResponse, type ProtocolResponseMap } from '../../sdk/typescript/codepet-pet-sdk/src/generated';
export type { PetSnapshot, PetTask, PetSource, PetTaskStatus } from '../../sdk/typescript/codepet-pet-sdk/src/generated';
export async function petRequest<M extends ProtocolMethod>(method: M, params: ProtocolRequestMap[M]): Promise<ProtocolResponseMap[M]> {
  const id = crypto.randomUUID();
  const result = await invoke<ProtocolResponse>('pet_gateway_request', { request: { protocolVersion: PROTOCOL_VERSION, id, method, params } });
  if (result.id !== id || result.method !== method || result.protocolVersion !== PROTOCOL_VERSION) throw new Error('Pet Gateway response mismatch');
  if (result.response.status === 'error') throw new Error(result.response.error.message);
  return result.response.result as ProtocolResponseMap[M];
}
export async function petSnapshot() { return (await petRequest('pet.snapshot', {})).snapshot; }
export const petStatusLabel = (status: string) => ({ idle: '空闲', running: '进行中', 'waiting-approval': '等待授权', 'waiting-input': '等待输入', completed: '本轮结束', failed: '失败', interrupted: '已取消', unknown: '状态待确认' }[status] ?? '状态待确认');
