<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { SidebarWidthController } from './sidebar';

  let {
    controller,
    controls,
    hidden = false,
  }: {
    controller: SidebarWidthController;
    controls: string;
    hidden?: boolean;
  } = $props();

  let currentWidth = $state(200);
  let currentBounds = $state({ min: 164, max: 280 });
  let isResizing = $state(false);
  const hintId = $props.id();
  let handle: HTMLDivElement | undefined = $state();
  let drag: { pointerId: number; x: number; width: number } | undefined;

  $effect(() => {
    const unsubWidth = controller.width.subscribe((value) => { currentWidth = value; });
    const unsubBounds = controller.bounds.subscribe((value) => { currentBounds = value; });
    const unsubResizing = controller.resizing.subscribe((value) => { isResizing = value; });
    return () => {
      unsubWidth();
      unsubBounds();
      unsubResizing();
    };
  });

  function finish(cancel = false) {
    if (!drag) return;
    const previous = drag;
    drag = undefined;
    if (cancel) controller.setWidth(previous.width, false);
    else controller.setWidth(currentWidth, true);
    if (handle?.hasPointerCapture(previous.pointerId)) handle.releasePointerCapture(previous.pointerId);
    controller.setResizing(false);
  }

  function start(event: PointerEvent) {
    if (event.button !== 0 || !event.isPrimary || hidden || !handle) return;
    event.preventDefault();
    drag = { pointerId: event.pointerId, x: event.clientX, width: currentWidth };
    handle.setPointerCapture(event.pointerId);
    controller.setResizing(true);
  }

  function keydown(event: KeyboardEvent) {
    const step = event.shiftKey ? 40 : 12;
    let next: number;
    if (event.key === 'ArrowRight') next = currentWidth + step;
    else if (event.key === 'ArrowLeft') next = currentWidth - step;
    else if (event.key === 'Home') next = currentBounds.min;
    else if (event.key === 'End') next = currentBounds.max;
    else return;
    event.preventDefault();
    controller.setWidth(next, true);
  }

  $effect(() => {
    if (hidden) finish(true);
  });
  onDestroy(() => finish(true));
</script>

<svelte:window
  onblur={() => finish(true)}
  onresize={() => finish(true)}
  onkeydown={(event) => {
    if (event.key === 'Escape' && drag) {
      event.preventDefault();
      finish(true);
    }
  }}
/>

{#if !hidden}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (A focusable separator implements the ARIA window splitter keyboard pattern.) -->
  <div
    class="sidebar-resize-handle"
    class:dragging={isResizing}
    bind:this={handle}
    role="separator"
    tabindex="0"
    aria-label="调整导航栏宽度"
    aria-orientation="vertical"
    aria-controls={controls}
    aria-describedby={hintId}
    aria-valuemin={currentBounds.min}
    aria-valuemax={currentBounds.max}
    aria-valuenow={currentWidth}
    aria-valuetext={`${Math.round(currentWidth)} 像素`}
    title="拖动调整导航栏宽度，双击恢复默认宽度"
    onpointerdown={start}
    onpointermove={(event) => {
      if (drag?.pointerId === event.pointerId) {
        controller.setWidth(drag.width + (event.clientX - drag.x), false);
      }
    }}
    onpointerup={(event) => {
      if (drag?.pointerId === event.pointerId) finish();
    }}
    onpointercancel={(event) => {
      if (drag?.pointerId === event.pointerId) finish(true);
    }}
    onlostpointercapture={() => finish(true)}
    ondblclick={() => controller.resetWidth()}
    onkeydown={keydown}
  >
    <span id={hintId} class="sr-only">左右拖动或使用左右方向键调整导航栏宽度，按住 Shift 加快调整；Home 最窄，End 最宽，拖动时按 Escape 取消。</span>
  </div>
{/if}

<style>
  .sidebar-resize-handle {
    position: absolute;
    inset: 8px 0 8px auto;
    width: 8px;
    z-index: 4;
    cursor: col-resize;
    touch-action: none;
    user-select: none;
    outline: none;
  }
  .sidebar-resize-handle::after {
    content: '';
    position: absolute;
    inset: 4px 2px 4px auto;
    width: 2px;
    border-radius: 2px;
    background: transparent;
    transition: background-color 140ms ease;
  }
  .sidebar-resize-handle:hover::after {
    background: var(--color-main-selected);
  }
  .sidebar-resize-handle.dragging::after,
  .sidebar-resize-handle:focus-visible::after {
    background: var(--color-main-press);
  }
</style>
