<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { onMount, tick } from 'svelte';
  import { Plus, RotateCcw, Unlink, X } from '@lucide/svelte';
  import petIcon from '../../../src-tauri/icons/32x32.png';
  import { defaultDrawerSize, drawerCommand, serializeDrawer, type DrawerWindow, type DrawerPointer, type DrawerRect, type DrawerSelectionOperation } from './windowDrawer';
  export let syncHitRegions: () => Promise<void>;
  export let followWindow: (frame: DrawerRect | null, inset?: {x:number;y:number}) => void;
  export let frame: HTMLElement;
  export let panel: HTMLElement;
  export let backdrop: HTMLElement;
  export let body: HTMLElement;
  export let host: HTMLElement | undefined;
  export let selected: DrawerWindow | null = null;
  export let helpOpen = false;
  export let visibleRects: DrawerRect[] = [];
  export let maxWidth: number;
  export let maxHeight: number;
  export let pointer: DrawerPointer | null;
  export let pinned = false;
  export let resizing = false;
  export let openPanel: () => void;
  export let failed: (error: unknown) => void;
  let windows: DrawerWindow[] = [];
  let activateNext = false;
  let pressed = false, tracking = false;
  export let dropReady = false;
  let probing = false, gesture = 0;
  let lastExternalRelease: number | undefined;
  let enterTimer: ReturnType<typeof setTimeout> | undefined;
  let error = '', busy = false, visible = false, disposed = false;
  let edge: HTMLButtonElement, bar: HTMLElement, helpPanel: HTMLElement;
  let timer: ReturnType<typeof setTimeout>, check: ReturnType<typeof setTimeout> | undefined;
  let dirty = false, immediateCheck = false, epoch = 0, lastPath = '';
  const idleCheckMs = 500;
  let awaitingSince: number | undefined;
  let coveredSince: number | undefined;
  let syncError = '';
  function nativeHeld() { return pointer?.pressed !== false; }
  function cancelCheck() { clearTimeout(check); check = undefined; immediateCheck = false; }
  function schedule(delay = 0) {
    if (disposed || suspended || !selected) return;
    if (syncing) { dirty = true; return; }
    if (immediateCheck) return;
    cancelCheck();
    immediateCheck = delay === 0;
    check = setTimeout(() => { check = undefined; immediateCheck = false; void sync(); }, delay);
  }
  type DrawerChange = { cover: boolean; seal?: boolean; dx: number; dy: number };
  function changed({cover, seal, dx, dy}: DrawerChange) {
    if (!selected || disposed || suspended) return;
    if (cover || dx || dy) epoch++;
    if (seal || (cover && (dx || dy))) aperture();
    else if (cover) {
      // A focus notification alone says nothing about which pixels changed.
      // Revoke input until the stack is verified without flashing the backdrop.
      if (visibleRects.length) {
        visibleRects = [];
        void tick().then(syncHitRegions).catch(failed);
      }
    }
    else if (dx || dy) {
      // Until the AX move is confirmed, expose only the overlap between the
      // last verified pixels on screen and their destination in the panel.
      const rect = host?.getBoundingClientRect();
      aperture(rect, visibleRects.map(r => ({
        x: r.x + Math.max(0,-dx), y: r.y + Math.max(0,-dy),
        width: r.width - Math.abs(dx), height: r.height - Math.abs(dy),
      })).filter(r => r.width > 0 && r.height > 0));
    }
    schedule();
  }
  let presented = false, syncing = false, suspended = false;
  function reveal() { clearTimeout(timer); clearTimeout(enterTimer); enterTimer = undefined; visible = true; }
  function enter() {
    clearTimeout(timer);
    if (!visible && !enterTimer) enterTimer = setTimeout(reveal, 180);
  }
  $: if (pinned) reveal(); else if (visible && !over) deferHide();
  function deferHide() {
    clearTimeout(enterTimer); enterTimer = undefined;
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (!pinned && !helpOpen && !bar?.matches(':hover') && !bar?.querySelector(':focus-visible') && !edge?.matches(':focus-visible')) visible = false;
    }, 700);
  }
  function inside(node: HTMLElement | undefined, point: {x:number;y:number}) {
    if (!node || !node.getClientRects().length) return false;
    const r = node.getBoundingClientRect();
    return point.x >= r.left && point.x < r.right && point.y >= r.top && point.y < r.bottom;
  }
  let over = false;
  $: if (pointer) {
    const next = inside(edge,pointer) || (visible && inside(bar,pointer)) || (helpOpen && inside(helpPanel,pointer));
    if (next && !over) enter(); else if (!next && over) deferHide();
    over = next;
  }
  // The native press is recorded before WKWebView samples the pointer.
  // Probe only at the destination and require the actual window to have moved.
  function sampleDrag(point: DrawerPointer) {
    const down = Boolean(point.pressed);
    // Wake geometry reconciliation on global mouse-up without interpreting
    // the native content gesture or sending a drop/release command.
    if (pressed && !down) { followWindow(null); schedule(); }
    const released = point.externalReleases !== undefined && lastExternalRelease !== undefined &&
      lastExternalRelease !== point.externalReleases;
    lastExternalRelease = point.externalReleases;
    // Content is excluded from our gesture handling. Native content presses
    // also never increment the backend's external drop/release counter.
    const inNative = inside(host,point) && !inside(edge,point) &&
      !(visible && inside(bar,point)) && !(helpOpen && inside(helpPanel,point));
    if (inNative || released && point.releasePoint && inside(host,point.releasePoint)) {
      pressed = down; tracking = false; dropReady = false; gesture++;
      return;
    }
    const inPanel = inside(panel, point);
    if (down && !pressed) {
      gesture++;
      tracking = !inPanel;
    }
    if (!down || !inPanel) { dropReady = false; gesture++; }
    if (down && inPanel && !dropReady && !probing) {
      const ticket = gesture;
      probing = true;
      void drawerCommand('dragProbe').then(moved => {
        if (ticket === gesture && !disposed && moved) {
          tracking = true;
          dropReady = true;
        }
      }).catch(() => {}).finally(() => { probing = false; });
    }
    if (!down && (tracking && pressed || released)) {
      const atPanel = inside(panel, released && point.releasePoint ? point.releasePoint : point);
      if (atPanel) consumeDrop('drop', true);
      else if (windows.length) consumeDrop(tracking ? 'dragEnd' : 'dockClick', false);
      tracking = false;
    }
    pressed = down;
  }
  function consumeDrop(operation: DrawerSelectionOperation, reportFailure: boolean) {
    void serializeDrawer(() => drawerCommand(operation))
      .then(async target => {
        if (!target || disposed) return;
        openPanel();
        await choose(target);
      }).catch(e => { if (reportFailure && !disposed) failed(e); });
  }
  $: if (pointer && edge) sampleDrag(pointer);
  function aperture(rect?: DOMRect, parts: DrawerRect[] = [], updateHit = true, covered = false) {
    if (!panel || !backdrop) return;
    const previous = visibleRects;
    const regionsChanged = parts.length !== previous.length || parts.some((r,i) =>
      r.x !== previous[i].x || r.y !== previous[i].y || r.width !== previous[i].width || r.height !== previous[i].height);
    if (regionsChanged) {
      visibleRects = parts;
    }
    const hadHole = host?.hasAttribute('data-presented');
    host?.toggleAttribute('data-presented', Boolean(rect && parts.length));
    host?.toggleAttribute('data-covered', covered);
    host?.removeAttribute('data-missing');
    if (rect) host?.removeAttribute('data-native-paused');
    if (updateHit && (hadHole !== Boolean(host?.hasAttribute('data-presented')) || regionsChanged)) {
      void tick().then(() => { if (!disposed) return syncHitRegions(); }).catch(failed);
    }
    if (!rect || !parts.length) { if (lastPath) backdrop.style.clipPath = lastPath = ''; return; }
    const bounds = panel.getBoundingClientRect();
    const outline = `M0 0 H${bounds.width} V${bounds.height} H0 Z`;
    const holes = parts.map(r => {
      const x = rect.left - bounds.left + r.x, y = rect.top - bounds.top + r.y;
      return `M${x} ${y} h${r.width} v${r.height} h${-r.width} Z`;
    });
    const path = `path(evenodd, "${outline} ${holes.join(' ')}")`;
    if (path !== lastPath) backdrop.style.clipPath = lastPath = path;
  }
  async function coverBeforeMove() {
    const openHole = host?.hasAttribute('data-presented');
    host?.setAttribute('data-native-paused','');
    aperture(undefined, [], false);
    // Wait for both the opaque paint and the native input update before moving the window.
    const inputReady = tick().then(syncHitRegions);
    const paintReady = openHole && !document.hidden && new Promise<void>(done => {
      let raf = 0;
      const finish = () => { clearTimeout(timer); cancelAnimationFrame(raf); done(); };
      const timer = setTimeout(finish, 80);
      raf = requestAnimationFrame(() => { raf = requestAnimationFrame(finish); });
    });
    await Promise.all([inputReady, paintReady]);
    followWindow(null);
  }
  async function park() {
    if (!presented) { aperture(); return; }
    await coverBeforeMove();
    await drawerCommand('park');
    presented = false;
    resizing = false;
  }
  export async function beforeCollapse() {
    suspended = true; epoch++; cancelCheck();
    try { await serializeDrawer(park); helpOpen = false; visible = false; }
    catch (e) { suspended = false; schedule(); throw e; }
  }
  export function resumed() { suspended = false; schedule(); }
  export async function prepareReveal() {
    cancelCheck();
    await tick();
    await sync(true);
  }
  async function sync(handoff = false) {
    if (!selected || busy || syncing || disposed || suspended) return;
    if (!host || !handoff && (frame?.getAttribute('data-phase') !== 'open' || frame.hasAttribute('data-morphing'))) return;
    const rect = host.getBoundingClientRect();
    if (rect.height < 30 || rect.width < 30) return;
    syncing = true;
    const target = selected;
    const generation = epoch;
    let retryDelay = idleCheckMs;
    try {
      await serializeDrawer(async () => {
        if (disposed || suspended || busy || selected !== target) return;
        const op = activateNext ? 'activate' : 'show';
        activateNext = false;
        // Even a failed native command may have moved the window partway.
        presented = true;
        const layout = await drawerCommand(op, target.id, {
          x: rect.x, y: rect.y, width: rect.width, height: rect.height, maxWidth, maxHeight,
        });
        if ('pending' in layout) {
          retryDelay = 16;
          if (op === 'activate') activateNext = true;
          aperture();
          return;
        }
        syncError = '';
        presented = true;
        if (disposed || busy || selected !== target || suspended || generation !== epoch) return;
        resizing = layout.resizing || Boolean(layout.tracking);
        if (layout.tracking && nativeHeld()) {
          if (!layout.frame) throw new Error('窗口跟随缺少原生位置');
          // Surface remains the sole host writer. Use the native origin and
          // slot inset together, so changing width never recentres the shell.
          followWindow(layout.frame,{x:rect.x,y:rect.y});
          const width = Math.min(Math.floor(maxWidth),Math.max(30,Math.ceil(layout.width)));
          const height = Math.min(Math.floor(maxHeight),Math.max(30,Math.ceil(layout.height)));
          if (width !== target.requiredWidth || height !== target.requiredHeight) {
            const fitted = {...target,requiredWidth:width,requiredHeight:height};
            windows = windows.map(item=>item.id===target.id ? fitted : item);
            selected = fitted;
          }
          aperture(rect,layout.visible);
          await tick();
          const slot = host?.getBoundingClientRect();
          if (!disposed && !suspended && nativeHeld() && slot && selected?.id === target.id)
            followWindow(layout.frame,{x:slot.x,y:slot.y});
          retryDelay = 50;
          return;
        }
        followWindow(null);
        if (resizing) {
          const width = Math.min(Math.floor(maxWidth), Math.max(30,Math.ceil(layout.width)));
          const height = Math.min(Math.floor(maxHeight), Math.max(30,Math.ceil(layout.height)));
          if (Math.round(rect.width) === width && Math.round(rect.height) === height) aperture(rect,layout.visible);
          else aperture();
          if (width !== target.requiredWidth || height !== target.requiredHeight) {
            const fitted = { ...target, requiredWidth: width, requiredHeight: height };
            windows = windows.map(item => item.id === fitted.id ? fitted : item);
            selected = fitted;
          }
          // A final check catches mouse-up even when the owner omits AXResized.
          retryDelay = 50;
          return;
        }
        if (layout.width > maxWidth + 1 || layout.height > maxHeight + 1) {
          throw new Error(`${target.name}需要 ${Math.ceil(layout.width)} × ${Math.ceil(layout.height)} 点，当前展示区最多容纳 ${Math.floor(maxWidth)} × ${Math.floor(maxHeight)} 点。`);
        }
        if (layout.width > rect.width + 1 || layout.height > rect.height + 1) {
          aperture();
          const fitted = { ...target, requiredWidth: Math.max(target.requiredWidth ?? defaultDrawerSize.width, Math.ceil(layout.width)),
            requiredHeight: Math.max(target.requiredHeight ?? defaultDrawerSize.height, Math.ceil(layout.height)) };
          if (fitted.requiredWidth !== target.requiredWidth || fitted.requiredHeight !== target.requiredHeight) {
            windows = windows.map(item => item.id === target.id ? fitted : item);
            selected = fitted; retryDelay = 16;
          }
          return;
        }
        if (layout.awaitingSurface) {
          coveredSince = undefined;
          aperture();
          awaitingSince ??= performance.now();
          if (performance.now() - awaitingSince < 400) retryDelay = 50;
          else host?.setAttribute('data-missing', '');
          return;
        }
        awaitingSince = undefined;
        if (layout.visible.length) coveredSince = undefined;
        else coveredSince ??= performance.now();
        const covered = coveredSince !== undefined && performance.now() - coveredSince >= 400;
        if (!layout.visible.length && !covered) retryDelay = 50;
        // Retry the compositor briefly while focus/order changes settle.
        // Visibility checks never request application activation.
        aperture(rect, layout.visible, true, covered);
      });
    } catch (e) {
      if (disposed || busy || suspended || selected !== target || generation !== epoch) return;
      // A failed read/move is not consent to relinquish this window. Keep its
      // Dock entry and original geometry, close the aperture, and retry.
      aperture();
      host?.setAttribute('data-missing', '');
      awaitingSince = coveredSince = undefined;
      const message = String(e);
      if (message !== syncError) { syncError = message; failed(e); }
    } finally {
      syncing = false;
      const delay = dirty ? 0 : retryDelay;
      dirty = false;
      schedule(delay);
    }
  }
  async function choose(target: DrawerWindow | null) {
    if (busy) return;
    busy = true;
    try {
      if (target) {
        const targetId = target.id;
        const existing = windows.find(item => item.id === targetId);
        if (existing) target = existing;
        else windows = [...windows, target];
      }
      if (selected?.id !== target?.id) await serializeDrawer(park);
      activateNext = Boolean(target);
      awaitingSince = coveredSince = undefined;
      syncError = '';
      selected = target; helpOpen = false; suspended = false; error = '';
    } catch (e) { failed(e); }
    finally { busy = false; schedule(); deferHide(); }
  }
  function activateSelected() { if (selected) void choose(selected); }
  async function dragHelp() {
    if (busy) return;
    helpOpen = true; reveal(); busy = true; error = '';
    try { await drawerCommand('permission'); }
    catch (e) { error = String(e); }
    finally { busy = false; schedule(); }
  }
  async function restoreWindow(id: string) {
    await serializeDrawer(async () => {
      await coverBeforeMove();
      await drawerCommand('release', id);
    });
    forgetWindow(id);
  }
  function forgetWindow(id: string) {
    windows = windows.filter(item => item.id !== id);
    if (selected?.id === id) {
      aperture();
      selected = null;
      activateNext = false;
      presented = false;
      resizing = false;
    }
  }
  async function release() {
    if (!selected || busy) return;
    busy = true;
    const id = selected.id;
    try {
      await restoreWindow(id);
    } catch (e) { failed(e); }
    finally { busy = false; schedule(); }
  }
  function step(direction: number) {
    const items = [null, ...windows];
    const index = items.findIndex(item => item?.id === selected?.id);
    const next = items[Math.max(0, Math.min(items.length - 1, index + direction))];
    if (next !== selected) void choose(next);
  }
  let lastWheel = 0, distance = 0, switched = false;
  function wheel(e: WheelEvent) {
    if (helpOpen || !windows.length || Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault();
    const now = performance.now();
    if (now - lastWheel > 180) { distance = 0; switched = false; }
    lastWheel = now;
    distance += e.deltaX * (e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? bar.clientWidth : 1);
    if (!switched && Math.abs(distance) >= 24) {
      switched = true;
      step(Math.sign(distance));
    }
  }
  function key(e: KeyboardEvent) {
    if(e.key === 'Escape' && (helpOpen || visible)) {
      e.stopPropagation(); helpOpen = false; edge.focus(); visible = pinned;
    }
    if(!helpOpen && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault();
      step(e.key === 'ArrowLeft' ? -1 : 1);
    }
  }
  onMount(() => {
    void serializeDrawer(() => drawerCommand('reset')).catch(failed);
    const phase = new MutationObserver(() => {
      if (frame.dataset.phase === 'open' && !frame.hasAttribute('data-morphing')) schedule();
      else { cancelCheck(); epoch++; aperture(); }
    });
    phase.observe(frame,{attributes:true,attributeFilter:['data-phase','data-morphing']});
    const resize = new ResizeObserver(() => schedule());
    resize.observe(body);
    let unlisten: (() => void) | undefined;
    void listen<DrawerChange>('pet-drawer-changed', event => changed(event.payload)).then(remove => {
      if (disposed) remove(); else unlisten = remove;
    }).catch(failed);
    return () => {
      disposed = true; cancelCheck(); phase.disconnect(); resize.disconnect(); unlisten?.(); clearTimeout(timer); clearTimeout(enterTimer); aperture(); followWindow(null);
      void serializeDrawer(() => drawerCommand('reset')).catch(failed);
    };
  });
</script>

<button bind:this={edge} class="edge" data-window-control aria-label="显示窗口 Dock" aria-expanded={visible} on:pointerenter={enter} on:pointerleave={deferHide} on:focus={reveal} on:blur={deferHide} on:click={reveal}></button>
{#if visible || helpOpen}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <nav bind:this={bar} class="window-dock" data-window-control aria-label="窗口 Dock" on:pointerenter={enter} on:pointerleave={deferHide} on:wheel|nonpassive={wheel} on:keydown={key} on:focusin={reveal} on:focusout={deferHide}>
    <button aria-label="Code Pet 通知列表" title="Code Pet 通知列表" aria-pressed={!selected} on:click={() => choose(null)}><img src={petIcon} alt="" /></button>
    <div class="items">
      {#each windows as item (item.id)}
        <button title={item.name + ' · ' + item.title} aria-pressed={item.id === selected?.id} on:click={() => choose(item)}>{#if item.icon}<img src={item.icon} alt={item.name}/>{:else}<span>{item.name.slice(0,2)}</span>{/if}</button>
      {/each}
    </div>
    <button aria-label="拖入窗口说明" aria-expanded={helpOpen} title="拖动窗口标题栏到桌宠以钉住" on:click={() => helpOpen ? (helpOpen=false) : dragHelp()}><Plus size={15}/></button>
    {#if selected}<button title="重新显示当前窗口" aria-label="重新显示当前窗口" on:click={activateSelected}><RotateCcw size={15}/></button><button title="释放当前窗口" aria-label="释放当前窗口" on:click={release}><Unlink size={15}/></button>{/if}
  </nav>
{/if}
{#if helpOpen}
  <!-- Escape also closes the drag instructions from the keyboard. -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <section bind:this={helpPanel} class="drag-help" data-window-control aria-label="拖入钉住窗口" on:keydown={key}>
    <header>拖入钉住 <button aria-label="关闭说明" on:click={() => {helpOpen=false;deferHide();}}><X size={13}/></button></header>
    {#if error}<p role="status">{error}</p><button on:click={() => drawerCommand('settings').catch(failed)}>打开辅助功能设置</button><button on:click={dragHelp}>刷新</button>
    {:else}<p>拖动应用窗口的标题栏，在桌宠面板上松手即可钉住。</p>{/if}
  </section>
{/if}
<style>
  .edge,.window-dock,.drag-help {position:absolute;z-index:4;left:50%;transform:translateX(-50%)}
  .edge {bottom:0;width:min(260px,75%);height:12px;border:0;background:transparent}
  .edge:focus-visible {outline:1px solid var(--gray-9)}
  .window-dock {bottom:5px;display:flex;align-items:center;gap:4px;max-width:95%;padding:4px 6px;background:var(--gray-3);color:var(--gray-12);border:1px solid var(--gray-7);border-radius:12px;box-shadow:0 4px 16px #0006}
  button {color:inherit;font:inherit;cursor:pointer}
  .window-dock button,.drag-help button {background:transparent;border:0;border-radius:6px}
  .window-dock button {display:grid;place-items:center;min-width:27px;height:27px;font-size:var(--font-size-xs)}
  .window-dock button[aria-pressed="true"],button:hover {background:var(--gray-5)}
  img {width:20px;height:20px;border-radius:4px}
  .items {display:flex;min-width:0;overflow-x:auto}
  .items:empty {display:none}
  .drag-help {bottom:44px;max-height:calc(100% - 50px);overflow:auto;width:min(280px,95%);padding:8px;background:var(--gray-3);color:var(--gray-12);border:1px solid var(--gray-7);border-radius:9px;font-size:var(--font-size-sm);box-sizing:border-box}
  header {display:flex;align-items:center;gap:6px}
  header {justify-content:space-between;font-weight:var(--font-weight-semibold)}
</style>
