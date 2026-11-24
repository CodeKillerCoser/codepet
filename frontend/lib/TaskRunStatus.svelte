<script lang="ts">
  import {tick} from 'svelte';
  import {ChevronDown} from '@lucide/svelte';
  import TaskStatus from './TaskStatus.svelte';
  import {statuses,type RunStatus} from './taskLedger';
  export let status:RunStatus;
  export let current=false;
  export let busy=false;
  export let onComplete:()=>void;
  export let onRestore:(button:HTMLButtonElement)=>void;
  let open=false,root:HTMLDivElement,trigger:HTMLButtonElement;
  let next:RunStatus|undefined;
  $: next=current?(status==='ready_for_delivery'?'closed':['closed','published'].includes(status)?'in_progress':undefined):undefined;
  $: if(busy||!next)open=false;
  async function toggle(){open=!open;if(open){await tick();root.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();}}
  function close(){open=false;trigger?.focus({preventScroll:true});}
  function choose(){close();if(next==='closed')onComplete();else onRestore(trigger);}
  function escape(event:KeyboardEvent){if(open&&event.key==='Escape'){event.preventDefault();event.stopPropagation();close();}}
</script>

<svelte:window on:click={event=>{if(open&&event.target instanceof Node&&!root.contains(event.target))open=false;}}/>
<div class="run-status-control" bind:this={root}>
  {#if next}
    <button class="quiet run-status-trigger" aria-label={`执行状态：${statuses[status]}`} aria-haspopup="menu" aria-expanded={open}
      disabled={busy} bind:this={trigger} on:click={toggle} on:keydown={event=>{escape(event);if(['ArrowDown','ArrowUp'].includes(event.key)){event.preventDefault();if(!open)void toggle();}}}>
      <TaskStatus {status}/><ChevronDown size={12}/>
    </button>
    {#if open}
      <div class="status-menu" role="menu" tabindex="-1" aria-label="修改执行状态" on:keydown={event=>{escape(event);if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){event.preventDefault();root.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();}if(event.key==='Tab')close();}}>
        <button role="menuitem" on:click={choose}><TaskStatus status={next}/></button>
      </div>
    {/if}
  {:else}<TaskStatus {status}/>{/if}
</div>

<style>
 .run-status-control{position:relative;display:inline-flex;align-items:center;}
 .run-status-control .run-status-trigger{display:inline-flex;align-items:center;gap:6px;padding:4px 7px;border:0;background:var(--wb-control);}
 .status-menu{position:absolute;top:calc(100% + 5px);left:0;z-index:4;min-width:130px;padding:4px;border:1px solid var(--wb-border-strong);border-radius:6px;background:var(--wb-panel);}
 .run-status-control .status-menu button{display:flex;align-items:center;width:100%;padding:6px 8px;text-align:left;border:0;background:transparent;}
 .run-status-control .status-menu button:hover,.run-status-control .status-menu button:focus-visible{background:var(--wb-control);}
</style>
