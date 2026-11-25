<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { DRAWER_DEFAULT_RATIO, useSidePanel, type SidePanelMode } from './sidePanel';

  let { controls, variant = 'column' }: { controls: string; variant?: SidePanelMode } = $props();
  const panel = useSidePanel();
  const width = panel.width, bounds = panel.bounds, current = panel.current, mode = panel.mode;
  const drawerWidth = panel.drawerWidth, drawerBounds = panel.drawerBounds;
  const currentWidth = $derived(variant === 'drawer' ? $drawerWidth : $width);
  const currentBounds = $derived(variant === 'drawer' ? $drawerBounds : $bounds);
  const setWidth = (next: number) => variant === 'drawer' ? panel.setDrawerWidth(next) : panel.setWidth(next);
  const resetWidth = () => variant === 'drawer' ? panel.setDrawerRatio(DRAWER_DEFAULT_RATIO) : panel.resetWidth();
  const hintId = $props.id();
  let handle: HTMLDivElement;
  let drag: { pointerId: number; x: number; width: number; mode: SidePanelMode; ratio: number } | undefined;

  function finish(cancel = false) {
    if (!drag) return;
    const previous = drag;
    drag = undefined;
    if (cancel) {
      if (previous.mode === 'drawer') panel.setDrawerRatio(previous.ratio);
      else panel.setWidth(previous.width);
    }
    if (handle.hasPointerCapture(previous.pointerId)) handle.releasePointerCapture(previous.pointerId);
    panel.setResizing(false);
  }

  function start(event: PointerEvent) {
    if (event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    handle.focus({ preventScroll: true });
    drag = { pointerId: event.pointerId, x: event.clientX, width: currentWidth, mode: variant, ratio: get(panel.drawerRatio) };
    handle.setPointerCapture(event.pointerId);
    panel.setResizing(true);
  }

  function keydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 50 : 16;
    let next: number;
    if (event.key === 'ArrowLeft') next = currentWidth + step;
    else if (event.key === 'ArrowRight') next = currentWidth - step;
    else if (event.key === 'Home') next = currentBounds.min;
    else if (event.key === 'End') next = currentBounds.max;
    else return;
    event.preventDefault();
    setWidth(next);
  }

  $effect(() => { if (!$current || $mode !== variant) finish(true); });
  onDestroy(() => finish(true));

</script>

<svelte:window onblur={() => finish(true)} onresize={() => finish(true)} onkeydown={(event) => {
  if (event.key === 'Escape' && drag) { event.preventDefault(); finish(true); }
}} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (A focusable separator implements the ARIA window splitter keyboard pattern.) -->
<div class="side-panel-resize-handle" bind:this={handle} role="separator" tabindex="0"
  aria-label="调整侧栏宽度" aria-orientation="vertical" aria-controls={controls} aria-describedby={hintId}
  aria-valuemin={currentBounds.min} aria-valuemax={currentBounds.max} aria-valuenow={currentWidth} aria-valuetext={`${Math.round(currentWidth)} 像素`}
  title="拖动调整侧栏宽度，双击恢复默认宽度"
  onpointerdown={start} onpointermove={(event) => {
    if (drag?.pointerId === event.pointerId) setWidth(drag.width + drag.x - event.clientX);
  }}
  onpointerup={(event) => { if (drag?.pointerId === event.pointerId) finish(); }}
  onpointercancel={(event) => { if (drag?.pointerId === event.pointerId) finish(true); }}
  onlostpointercapture={() => finish(true)} ondblclick={resetWidth}
  onkeydown={keydown}>
  <span id={hintId} class="sr-only">左右拖动或使用左右方向键调整宽度，按住 Shift 加快调整；Home 最窄，End 最宽，拖动时按 Escape 取消。</span>
</div>

<style>
  .side-panel-resize-handle { position: absolute; inset: 0 auto 0 0; width: 8px; cursor: col-resize; touch-action: none; user-select: none; outline: none; }
  .side-panel-resize-handle::after { content: ''; position: absolute; inset: 12px 3px; border-radius: 2px; background: transparent; }
  .side-panel-resize-handle:hover::after { background: var(--app-border-strong); }
  .side-panel-resize-handle:focus-visible::after { background: var(--color-focus-ring); }
</style>
