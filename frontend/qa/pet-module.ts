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
  monitors: [{ name: "QA", scaleFactor: 2, position: { x: 0, y: 0 }, size: { width: 2880, height: 1800 }, workArea: { position: { x: 0, y: 40 }, size: { width: 2880, height: 1720 } } }],
};
const qa = {
  native,
  surface: { revision: 1, mode: 'floating' as 'floating' | 'docked', dragging: false, snapCandidate: false, canDock: true, availableHeight: 860, availableWidth: 1440, nativeDrag: true, physicalNotch: true },
  manualDrag: false,
  finishDrag: (docked = false) => {
    qa.surface = { ...qa.surface, revision: qa.surface.revision + 1, dragging: false, snapCandidate: false, mode: docked ? 'docked' : 'floating' };
    fire('pet-surface-changed', qa.surface);
  },
  move: (x: number, y: number) => { native.position = { x, y };
    if (lastLayout) anchor = { x: x + lastLayout.anchor.x * native.scale, y: y + lastLayout.anchor.y * native.scale };
    qa.surface = { ...qa.surface, availableHeight: native.monitors[0].workArea.size.height / native.scale, revision: qa.surface.revision + 1 };
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
  drags: 0,
  hit: (x: number, y: number) => isPointOnOpaquePetImage(document.querySelector(".pet-window")!, { x, y }),
  updateSettings: () => {},
};
(window as any).__petQa = qa;
const listeners = new Map<string, number>();
function fire(event: string, payload: unknown) {
  const id = listeners.get(event);
  if (id !== undefined) (window as any).__TAURI_INTERNALS__.runCallback(id, { event, id, payload });
}
qa.updateSettings = () => fire("settings-updated", qa.settings);
let anchor: { x: number; y: number } | undefined;
let lastLayout: any;
mockWindows("pet");
mockIPC((command, args: any) => {
  if (command === "provider_connection_status") return qa.providers;
  if (command === "get_app_settings") return qa.settings;
  if (command === "pet_gateway_request") {
    const request = args.request;
    return { ...request, response: { status: "ok", result: { snapshot: { tasks: qa.tasks, sources: [] } } } };
  }
  if (command === "plugin:event|listen") { listeners.set(args.event, args.handler); return args.handler; }
  if (command === 'pet_surface_state') return qa.surface;
  if (command === 'pet_surface_pointer') return qa.cursor;
  if (command === 'pet_surface_mode') {
    qa.surface = { ...qa.surface, mode: args.mode, revision: qa.surface.revision + 1 };
    fire('pet-surface-changed', qa.surface);
    return qa.surface;
  }
  if (command === 'pet_surface_drag') {
    if (args.cancel) { qa.finishDrag(); return qa.surface; }
    qa.drags++;
    qa.surface = { ...qa.surface, mode: 'floating', dragging: true, revision: qa.surface.revision + 1 };
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
    if (qa.surface.mode === 'docked') { x = m.position.x + m.size.width / 2 - l.width * scale / 2; y = m.position.y + 40 * scale; }
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
