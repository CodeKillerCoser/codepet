(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  Object.defineProperty(document, 'hasFocus', { configurable:true, value:()=>true });
  const results=[];
  for (const [name,expanded,reverse] of [['open',true,false],['close',false,false],['reverse',true,true]]) {
    const frames=[], started=performance.now();
    const sample=async()=>{
      const native=await invoke('qa_motion_frame');
      const live=document.querySelector('main'), content=live.querySelector('[data-dock-frame]');
      const visual=live;
      const rect=s=>{const r=visual.querySelector(s).getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
      const dom={screen:[screenX,screenY,outerWidth,outerHeight],t:performance.now()-started,viewport:[innerWidth,innerHeight],avatar:rect('.pet-avatar'),background:rect('.notch-backdrop'),morphing:content.hasAttribute('data-morphing')};
      frames.push({...dom,native});
    };
    await sample();
    const panel=document.querySelector('main .notch-panel');
    panel.dispatchEvent(new MouseEvent('mouseenter'));
    panel.querySelector('.notch-summary').click();
    if(reverse)setTimeout(()=>panel.querySelector('.notch-summary').click(),110);
    while(performance.now()-started<700){await new Promise(requestAnimationFrame);await sample();}
    const phase=document.querySelector('[data-dock-frame]').dataset.phase;
    if (phase !== (expanded && !reverse ? 'open' : 'closed')) throw new Error(`${name}: unfinished transition (${phase})`);
    results.push({name,frames});
    if(frames.some(f=>Math.abs(f.native.width-f.viewport[0])>1)) throw new Error(`${name}: native/WebView frame mismatch`);
    if(frames.some(f=>Math.abs(f.avatar.width-frames[0].avatar.width)>.1)) throw new Error(`${name}: avatar was scaled`);
    for(let i=1;i<frames.length;i++) {
      const a=frames[i-1],b=frames[i];
      const dx=Math.abs(b.native.x+b.avatar.x-a.native.x-a.avatar.x);
      if(dx*Math.min(1,16.67/(b.t-a.t))>45) throw new Error(`${name}: discontinuous movement ${dx}px`);
    }
  }
  await invoke('qa_motion_done',{value:results});
})().catch(error=>window.__TAURI_INTERNALS__.invoke('qa_motion_done',{value:{error:String(error)}}));
