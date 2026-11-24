<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { SIDE_PANEL_DEFAULT_WIDTH, useSidePanel } from './sidePanel';

  let { controls }: { controls: string } = $props();
  const panel = useSidePanel();
  const width = panel.width, bounds = panel.bounds, current = panel.current, mode = panel.mode;
  const hintId = $props.id();
  let handle: HTMLDivElement;
  let drag: { pointerId: number; x: number; width: number } | undefined;

  function finish(cancel = false) {
    if (!drag) return;
    const previous = drag;
    drag = undefined;
    if (cancel) panel.setWidth(previous.width);
    if (handle.hasPointerCapture(previous.pointerId)) handle.releasePointerCapture(previous.pointerId);
    panel.setResizing(false);
  }

  function start(event: PointerEvent) {
    if (event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    handle.focus({ preventScroll: true });
    drag = { pointerId: event.pointerId, x: event.clientX, width: get(width) };
    handle.setPointerCapture(event.pointerId);
    panel.setResizing(true);
  }

  function keydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 50 : 16;
    let next: number;
    if (event.key === 'ArrowLeft') next = $width + step;
    else if (event.key === 'ArrowRight') next = $width - step;
    else if (event.key === 'Home') next = $bounds.min;
    else if (event.key === 'End') next = $bounds.max;
    else return;
    event.preventDefault();
    panel.setWidth(next);
  }

  $effect(() => { if (!$current || $mode !== 'column') finish(true); });
  onDestroy(() => finish(true));
  onMount(() => {
    const layout = handle.closest<HTMLElement>('[data-side-panel-layout]');
    if (!layout) return;
    const navigation = layout.querySelector<HTMLElement>(':scope > .sidebar');
    const measure = () => {
      const style = getComputedStyle(layout);
      panel.setAvailableWidth(layout.clientWidth - parseFloat(style.paddingLeft || '0')
        - parseFloat(style.paddingRight || '0') - (navigation?.getBoundingClientRect().width ?? 0));
    };
    const resize = new ResizeObserver(measure);
    const classes = new MutationObserver(measure);
    const transitionEnd = (event: TransitionEvent) => { if (event.target === layout) measure(); };
    resize.observe(layout);
    // A collapsed navigation changes available width even when the window stays
    // the same size. Measure its final layout, without chasing animated frames.
    classes.observe(layout, { attributes: true, attributeFilter: ['class'] });
    layout.addEventListener('transitionend', transitionEnd);
    measure();
    return () => { resize.disconnect(); classes.disconnect(); layout.removeEventListener('transitionend', transitionEnd); };
  });
</script>

<svelte:window onblur={() => finish(true)} onkeydown={(event) => {
  if (event.key === 'Escape' && drag) { event.preventDefault(); finish(true); }
}} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (A focusable separator implements the ARIA window splitter keyboard pattern.) -->
<div class="side-panel-resize-handle" bind:this={handle} role="separator" tabindex="0"
  aria-label="调整侧栏宽度" aria-orientation="vertical" aria-controls={controls} aria-describedby={hintId}
  aria-valuemin={$bounds.min} aria-valuemax={$bounds.max} aria-valuenow={$width} aria-valuetext={`${$width} 像素`}
  title="拖动调整侧栏宽度，双击恢复默认宽度"
  onpointerdown={start} onpointermove={(event) => {
    if (drag?.pointerId === event.pointerId) panel.setWidth(drag.width + drag.x - event.clientX);
  }}
  onpointerup={(event) => { if (drag?.pointerId === event.pointerId) finish(); }}
  onpointercancel={(event) => { if (drag?.pointerId === event.pointerId) finish(true); }}
  onlostpointercapture={() => finish(true)} ondblclick={() => panel.setWidth(SIDE_PANEL_DEFAULT_WIDTH)}
  onkeydown={keydown}>
  <span id={hintId} class="sr-only">左右拖动或使用左右方向键调整宽度，按住 Shift 加快调整；Home 最窄，End 最宽，拖动时按 Escape 取消。</span>
</div>

<style>
  .side-panel-resize-handle { position: absolute; inset: 0 auto 0 0; width: 8px; cursor: col-resize; touch-action: none; user-select: none; outline: none; }
  .side-panel-resize-handle::after { content: ''; position: absolute; inset: 12px 3px; border-radius: 2px; background: transparent; }
  .side-panel-resize-handle:hover::after { background: var(--app-border-strong); }
  .side-panel-resize-handle:focus-visible::after { background: var(--color-focus-ring); }
</style>
