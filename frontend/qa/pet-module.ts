// Production components with simulated native IPC; not part of release inputs.
import { mount } from "svelte";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import PetApp from "../PetApp.svelte";
import PetLibraryQa from "./PetLibraryQa.svelte";
import { builtInPets } from "../lib/pet/assets";
import { isPointOnOpaquePetImage } from "../lib/pet/hitTest";
import { defaultRunningBubbleSettings } from "../lib/theme";
import type { AppSettings } from "../lib/types";
import "../styles.css";

const settings = {
  appearance: { theme: "light", runningBubble: defaultRunningBubbleSettings },
  pet: { selectedPetId: "default", kind: "builtin", scale: 3, opacity: 1 },
  petLibrary: { selectedPetId: "default", pets: builtInPets.map(pet => ({ id: pet.id, name: pet.name, kind: "builtin", imagePath: null })) },
} as AppSettings;
const frameMode = new URLSearchParams(location.search).has("frame");
const native = {
  position: { x: 400, y: 300 },
  size: { width: 640, height: 1168 },
  scale: 2,
  resizeCount: 0,
  dockRatio: 0.5,
  monitors: [{ name: "QA", scaleFactor: 2, position: { x: 0, y: 0 }, size: { width: 2880, height: 1800 }, workArea: { position: { x: 0, y: 40 }, size: { width: 2880, height: 1720 } } }],
};
const qa = {
  native,
  surface: { revision: 1, mode: 'floating' as 'floating' | 'docked', dragging: false, snapCandidate: false, canDock: true, availableHeight: 860, availableWidth: 1440, nativeDrag: true, physicalNotch: true, topInset: 0, notchWidth: 185 },
  manualDrag: false,
  dragAlongTop: false,
  finishDrag: (docked = false) => {
    qa.surface = { ...qa.surface, revision: qa.surface.revision + 1, dragging: false, snapCandidate: false, mode: docked ? 'docked' : 'floating' };
    fire('pet-surface-changed', qa.surface);
  },
  move: (x: number, y: number) => { native.position = { x, y };
    if (lastLayout) anchor = { x: x + lastLayout.anchor.x * native.scale, y: y + lastLayout.anchor.y * native.scale };
    qa.surface = { ...qa.surface, availableWidth: native.monitors[0].workArea.size.width / native.scale, availableHeight: native.monitors[0].workArea.size.height / native.scale, revision: qa.surface.revision + 1 };
    fire('pet-surface-changed', qa.surface); },
  tasks: [] as { id: string; title: string; status: string; providerId: string; updatedAt: number }[],
  settings,
  providers: [
    { providerId: 'dev.codepet.codex', displayName: 'Codex', icon: 'https://avatars.githubusercontent.com/u/14957082?s=200&v=4', connectionStatus: 'online', generation: 1, instances: [] },
    { providerId: 'dev.codepet.claude', displayName: 'Claude Code', icon: 'https://cdn.prod.website-files.com/6889473510b50328dbb70ae6/68c33859cc6cd903686c66a2_apple-touch-icon.png', connectionStatus: 'online', generation: 1, instances: [] },
  ],
  assets: {} as Record<string, string>,
  ignored: false,
  cursor: { x: 0, y: 0 },
  cursorScreen: null as { x: number; y: number } | null,
  drags: 0,
  hit: (x: number, y: number) => isPointOnOpaquePetImage(document.querySelector(".pet-window")!, { x, y }),
  updateSettings: () => {},
  beforeSnapshot: null as (() => Promise<void>) | null,
};
(window as any).__petQa = qa;
const listeners = new Map<string, number>();
function fire(event: string, payload: unknown) {
  const id = listeners.get(event);
  if (id !== undefined) (window as any).__TAURI_INTERNALS__.runCallback(id, { event, id, payload });
}
// Presentation scenarios use concise fixtures; only the native mock translates
// them into the current Notice contract. No retired RPC exists in production.
let fixtureRevision=0;
function fixtureNotice(task:any){
  return {noticeId:task.id,pluginId:task.providerId.startsWith('dev.')?task.providerId:`dev.codepet.${task.providerId}`,sourceId:'qa',
    revision:fixtureRevision,finished:['completed','interrupted'].includes(task.status),
    status:{value:task.status,label:({'running':'进行中','completed':'本轮结束','interrupted':'已取消','waiting-input':'等待输入','waiting-approval':'等待授权','failed':'失败'}as Record<string,string>)[task.status]??task.status},
    content:{title:task.title,progress:task.summary?{text:task.summary}:undefined},
    actions:['waiting-input','waiting-approval','failed'].includes(task.status)?[{kind:'reply',id:'reply',label:'回复',enabled:true}]:[],
    createdAt:task.updatedAt,updatedAt:task.updatedAt};
}
let fixtureTasks=qa.tasks;
Object.defineProperty(qa,'tasks',{get:()=>fixtureTasks,set:(tasks)=>{
  fixtureTasks=tasks;fixtureRevision++;
  fire('provider-connection-status',[]);
  fire('provider-connection-status',qa.providers);
  for(const task of tasks)fire('codepet-gateway-event',{jsonrpc:'2.0',method:'notice.changed',params:{eventCursor:`qa-${fixtureRevision}-${task.id}`,payload:{notice:fixtureNotice(task)}}});
}});
qa.updateSettings = () => fire("settings-updated", qa.settings);
let anchor: { x: number; y: number } | undefined;
let lastLayout: any;
mockWindows("pet");
mockIPC((command, args: any) => {
  if (command === "provider_connection_status") return qa.providers;
  if (command === "get_app_settings") return qa.settings;
  if (command === "codepet_gateway_request") {
    const request=args.request;
    if(request.method !== 'notice.list') throw new Error(`Unexpected QA Gateway method ${request.method}`);
    const response=()=>({jsonrpc:'2.0',id:request.id,result:{notices:qa.tasks.map(fixtureNotice),sources:qa.providers.map(p=>({pluginId:p.providerId,displayName:p.displayName,icon:p.icon,enabled:true,connected:true,message:''})),hasMore:false}});
    return qa.beforeSnapshot ? qa.beforeSnapshot().then(response) : response();
  }
  if (command === "plugin:event|listen") { listeners.set(args.event, args.handler); return args.handler; }
  if (command === 'pet_surface_begin_frame') return 0;
  if (command === 'pet_surface_end_frame') return;
  if (command === 'pet_surface_state') return qa.surface;
  if (command === 'pet_surface_pointer') return qa.cursorScreen
    ? { x: qa.cursorScreen.x - native.position.x / native.scale, y: qa.cursorScreen.y - native.position.y / native.scale } : qa.cursor;
  if (command === 'pet_surface_plan_layout') {
    const l = args.layout, m = native.monitors[0], scale = native.scale;
    const x = Math.max(m.position.x, Math.min(m.position.x + m.size.width * native.dockRatio - l.width * scale / 2, m.position.x + m.size.width - l.width * scale));
    return { from: { x: native.position.x / scale, y: native.position.y / scale, width: native.size.width / scale, height: native.size.height / scale },
      to: { x: x / scale, y: m.position.y / scale + 40, width: l.width, height: l.height }, topInset: qa.surface.topInset };
  }
  if (command === 'pet_surface_mode') {
    qa.surface = { ...qa.surface, mode: args.mode, revision: qa.surface.revision + 1 };
    fire('pet-surface-changed', qa.surface);
    return qa.surface;
  }
  if (command === 'pet_surface_drag') {
    if (args.cancel) { qa.finishDrag(); return qa.surface; }
    qa.drags++;
    qa.surface = { ...qa.surface, mode: qa.dragAlongTop ? 'docked' : 'floating', dragging: true, revision: qa.surface.revision + 1 };
    fire('pet-surface-changed', qa.surface);
    if (!qa.manualDrag) setTimeout(() => qa.finishDrag(), 200);
    return qa.surface;
  }
  if (command === 'pet_surface_layout') {
    if (!frameMode || args.layout.revision !== qa.surface.revision) return qa.surface;
    const l = args.layout, scale = native.scale, m = native.monitors[0], area = m.workArea;
    anchor ??= { x: area.position.x + area.size.width - 95 * scale, y: area.position.y + area.size.height - 95 * scale };
    let x = anchor.x - l.anchor.x * scale, y = anchor.y - l.anchor.y * scale;
    const size = { width: Math.round(l.width * scale), height: Math.round(l.height * scale) };
    if (qa.surface.mode === 'docked') { x = Math.max(m.position.x, Math.min(m.position.x + m.size.width * native.dockRatio - l.width * scale / 2, m.position.x + m.size.width - size.width)); y = m.position.y + 40 * scale; }
    else if (!qa.surface.dragging) {
      x = Math.max(area.position.x, Math.min(x, area.position.x + area.size.width - size.width));
      y = Math.max(area.position.y, Math.min(y, area.position.y + area.size.height - size.height));
    }
    native.position = { x, y }; native.size = size; native.resizeCount++;
    anchor = { x: x + l.anchor.x * scale, y: y + l.anchor.y * scale }; lastLayout = l;
    return Promise.resolve((window as any).__resizeNativeWindow({ width: l.width, height: l.height })).then(() => qa.surface);
  }
  if (command === "plugin:window|set_ignore_cursor_events") { qa.ignored = args.value; return; }
  if (command === "plugin:window|start_dragging") { qa.drags++; return new Promise(resolve => setTimeout(resolve, 200)); }
  if (command === "pet_asset_data_url") { if (qa.assets[args.path]) return qa.assets[args.path]; throw new Error("missing test asset"); }
});
// Exercise the component below the main app shell's 320px minimum.
document.body.style.minWidth = "0";
const target = document.getElementById("app")!;
if (new URLSearchParams(location.search).has("library")) {
  void import("../main-window.css").then(() => mount(PetLibraryQa, { target, props: { initial: settings } }));
}
else mount(PetApp, { target });
