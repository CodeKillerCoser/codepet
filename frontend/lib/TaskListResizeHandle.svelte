<script lang="ts">
  import {onDestroy} from 'svelte';
  export let value:number;
  export let min=220;
  export let max=440;
  export let active=true;
  export let onResize:(value:number)=>void;
  let handle:HTMLDivElement;
  let drag:{id:number;x:number;width:number}|undefined;
  const clamp=(width:number)=>Math.round(Math.max(min,Math.min(max,width)));
  function finish(cancel=false){
    if(!drag)return;const previous=drag;drag=undefined;
    if(cancel)onResize(clamp(previous.width));
    if(handle?.hasPointerCapture(previous.id))handle.releasePointerCapture(previous.id);
  }
  function start(event:PointerEvent){
    if(!active||event.button!==0||!event.isPrimary)return;
    event.preventDefault();handle.focus({preventScroll:true});
    drag={id:event.pointerId,x:event.clientX,width:value};handle.setPointerCapture(event.pointerId);
  }
  function keydown(event:KeyboardEvent){
    if(!active)return;const step=event.shiftKey?50:16;
    const next=event.key==='ArrowLeft'?value-step:event.key==='ArrowRight'?value+step:event.key==='Home'?min:event.key==='End'?max:undefined;
    if(next===undefined)return;event.preventDefault();onResize(clamp(next));
  }
  $: if(!active)finish(true);
  onDestroy(()=>finish(true));
</script>
<svelte:window on:blur={()=>finish(true)} on:resize={()=>finish(true)} on:keydown|capture={event=>{if(drag&&event.key==='Escape'){event.preventDefault();event.stopPropagation();finish(true);}}}/>
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (Keyboard-operable window splitter.) -->
<div class="task-list-resizer" class:dragging={!!drag} bind:this={handle} role="separator" tabindex="0" aria-label="调整任务列表宽度" aria-orientation="vertical" aria-controls="task-ledger-list" aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} aria-valuetext={`${value} 像素`} title="拖动调整任务列表宽度，双击恢复默认"
 on:pointerdown={start} on:pointermove={event=>{if(drag?.id===event.pointerId)onResize(clamp(drag.width+event.clientX-drag.x));}}
 on:pointerup={event=>{if(drag?.id===event.pointerId)finish();}} on:pointercancel={()=>finish(true)} on:lostpointercapture={()=>finish(true)}
 on:dblclick={()=>onResize(clamp(280))} on:keydown={keydown}></div>
<style>
 .task-list-resizer{position:relative;width:8px;min-height:0;cursor:col-resize;touch-action:none;user-select:none;outline:none}
 .task-list-resizer::after{position:absolute;content:'';inset:0 3px;background:var(--wb-border)}
 .task-list-resizer:hover::after,.dragging::after{background:var(--wb-border-strong)}.task-list-resizer:focus-visible::after{background:var(--color-focus-ring)}
</style>
