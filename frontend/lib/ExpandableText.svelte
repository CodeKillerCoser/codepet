<script lang="ts">
  import {onMount,tick} from 'svelte';
  import {ChevronDown,ChevronUp} from '@lucide/svelte';
  export let text='';
  export let label='内容';
  export let textClass='';
  export let prefix='';
  export let reference='';
  export let lines=3;
  let paragraph:HTMLParagraphElement;
  let referenceElement:HTMLParagraphElement;
  let expanded=false,overflowing=false;

  function measure(){
    if(!paragraph?.isConnected||!paragraph.clientWidth)return;
    const lineHeight=parseFloat(getComputedStyle(paragraph).lineHeight);
    const referenceOverflow=referenceElement&&(referenceElement.scrollWidth>referenceElement.clientWidth+1||referenceElement.scrollHeight>parseFloat(getComputedStyle(referenceElement).lineHeight)+1);
    overflowing=paragraph.scrollHeight>lineHeight*lines+1||Boolean(referenceOverflow);
  }
  $: if(text||reference||lines)void tick().then(measure);
  onMount(()=>{
    const observer=new ResizeObserver(measure);
    observer.observe(paragraph);
    if(referenceElement)observer.observe(referenceElement);
    void document.fonts.ready.then(measure);
    measure();
    return()=>observer.disconnect();
  });
</script>

<p bind:this={paragraph} class={textClass} class:reading-clamped={!expanded} style:--reading-lines={lines}>{#if prefix}<span class="reading-prefix">{prefix}</span>{/if}{text}</p>
{#if reference}<p bind:this={referenceElement} class="reading-reference" class:reference-clamped={!expanded}>{reference}</p>{/if}
{#if overflowing||expanded}
  <button class="quiet reading-toggle" aria-label={`${expanded?'收起':'展开'}${label}`} aria-expanded={expanded} title={expanded?'收起':'展开'} on:click={()=>expanded=!expanded}>{#if expanded}<ChevronUp size={15}/>{:else}<ChevronDown size={15}/>{/if}</button>
{/if}

<style>
 p{margin:0;font:inherit;white-space:pre-wrap;overflow-wrap:anywhere;}
 .reading-clamped{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:var(--reading-lines);line-clamp:var(--reading-lines);overflow:hidden;}
 .reading-prefix{margin-right:var(--space-inline);color:var(--task-secondary);}
 .reading-reference{margin-top:var(--space-inline);font:var(--font-caption);color:var(--task-secondary);}
 .reference-clamped{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
 .reading-toggle{display:grid;place-items:center;padding:0;width:24px;min-height:24px;margin-top:var(--space-inline);border:0;background:transparent;}
</style>
