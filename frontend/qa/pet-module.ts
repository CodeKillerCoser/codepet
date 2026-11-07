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
  move: (x: number, y: number) => { native.position = { x, y }; fire("tauri://move", native.position); },
  tasks: [] as { id: string; title: string; status: string; providerId: string; updatedAt: number }[],
  settings,
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
mockWindows("pet");
mockIPC((command, args: any) => {
  if (command === "get_app_settings") return qa.settings;
  if (command === "pet_gateway_request") {
    const request = args.request;
    return { ...request, response: { status: "ok", result: { snapshot: { tasks: qa.tasks, sources: [] } } } };
  }
  if (command === "plugin:event|listen") { listeners.set(args.event, args.handler); return args.handler; }
  if (frameMode) {
    if (command === "plugin:window|inner_size" || command === "plugin:window|outer_size") return native.size;
    if (command === "plugin:window|outer_position") return native.position;
    if (command === "plugin:window|scale_factor") return native.scale;
    if (command === "plugin:window|available_monitors") return native.monitors;
    if (command === "plugin:window|primary_monitor") return native.monitors[0];
    if (command === "plugin:window|set_size") {
      const physical = args.value.toPhysical(native.scale);
      native.size = { width: Math.round(physical.width), height: Math.round(physical.height) };
      native.resizeCount++;
      fire("tauri://resize", native.size);
      const logical = args.value.toLogical(native.scale);
      return (window as any).__resizeNativeWindow({ width: Math.round(logical.width), height: Math.round(logical.height) });
    }
    if (command === "plugin:window|set_position") {
      native.position = args.value.toPhysical(native.scale);
      fire("tauri://move", native.position);
      return;
    }
  }
  if (command === "plugin:window|cursor_position") return qa.cursor;
  if (command === "plugin:window|outer_position") return { x: 0, y: 0 };
  if (command === "plugin:window|outer_size") return { width: 320, height: 584 };
  if (command === "plugin:window|scale_factor") return 1;
  if (command === "plugin:window|available_monitors") return [];
  if (command === "plugin:window|primary_monitor") return null;
  if (command === "plugin:window|set_ignore_cursor_events") { qa.ignored = args.value; return; }
  if (command === "plugin:window|start_dragging") { qa.drags++; return new Promise(resolve => setTimeout(resolve, 200)); }
  if (command === "pet_asset_data_url") throw new Error("missing test asset");
});
// Exercise the component below the main app shell's 320px minimum.
document.body.style.minWidth = "0";
const target = document.getElementById("app")!;
if (new URLSearchParams(location.search).has("library")) {
  void import("../main-window.css").then(() => mount(PetLibraryQa, { target, props: { initial: settings } }));
}
else mount(PetApp, { target });
