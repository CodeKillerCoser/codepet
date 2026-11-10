// Per-frame screen-coordinate regression for the production top-panel components.
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { build, preview } from 'vite';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.CODEPET_QA_NODE_MODULES ? resolve(process.env.CODEPET_QA_NODE_MODULES, 'playwright') : 'playwright');
const output = resolve('artifacts/pet-motion-qa'), outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1436, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ channel: process.env.CODEPET_QA_BROWSER ?? 'chromium', headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.setDefaultTimeout(15000);
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  const resizeClient = await page.context().newCDPSession(page);
  await page.exposeFunction('__resizeNativeWindow', async size => { await resizeClient.send('Emulation.setDeviceMetricsOverride',{...size,deviceScaleFactor:1,mobile:false,dontSetVisibleSize:true}); });
  await page.goto('http://127.0.0.1:1436/frontend/qa/pet-module.html?frame');
  await page.waitForSelector('.pet-avatar img');
  // Compositor screenshots can trail rAF/IPC by a frame. Encode the viewport in
  // two test-only pixels so each image is aligned using its own rendered frame.
  await page.evaluate(() => {
    const marker=document.createElement('div');
    marker.style.cssText='position:fixed;left:0;top:0;width:2px;height:1px;z-index:2147483647;pointer-events:none';
    document.body.append(marker);
    const update=()=>marker.style.background=`linear-gradient(to right,rgb(${innerWidth>>8},${innerWidth&255},0) 0 50%,rgb(${innerHeight>>8},${innerHeight&255},0) 50% 100%)`;
    window.addEventListener('resize',update);update();
  });
  await page.evaluate(() => {
    const q = window.__petQa;
    q.tasks = [{ id:'motion', title:'验证顶部展开动画的连续性', summary:'头像和标题保持原尺寸\n逐帧检查窗口和内容的位置', providerId:'codex', status:'running', updatedAt:Date.now() }];
    q.cursor = { x:-20,y:-20 };
    q.finishDrag(true);
  });
  await page.waitForFunction(() => innerHeight < 60 && document.querySelector('.latest-title')?.textContent.includes('验证'));
  await page.mouse.move(-20,-20);
  const client = await page.context().newCDPSession(page);
  const screenshots = [];
  let recording = '';
  client.on('Page.screencastFrame', async event => {
    await client.send('Page.screencastFrameAck', {sessionId:event.sessionId});
    if (!recording) return;
    const name = `${recording}-${String(screenshots.length).padStart(3,'0')}.png`;
    screenshots.push({name, metadata:event.metadata});
    await writeFile(resolve(output,name),Buffer.from(event.data,'base64'));
  });
  const results = [];
  for (const [name,ratio,expanded,reverse] of [
    ['center-open',.5,true,false], ['center-close',.5,false,false],
    ['left-open',.04,true,false], ['left-close',.04,false,false],
    ['right-open',.96,true,false], ['right-close',.96,false,false],
    ['reverse',.5,true,100], ['rapid-reverse',.5,true,1],
  ]) {
    await page.evaluate(r => { const q=window.__petQa; q.native.dockRatio=r; q.finishDrag(true); },ratio);
    await page.waitForTimeout(100);
    recording = name;
    await client.send('Page.startScreencast',{format:'png',everyNthFrame:1,maxWidth:1440,maxHeight:900});
    const frames = await page.evaluate(async ({expanded,reverse}) => {
      const frames = [], started = performance.now();
      const read = el => {
        if(!el) return null;
        const r=el.getBoundingClientRect(), s=getComputedStyle(el), q=window.__petQa, m=q.native.monitors[0], scale=q.native.scale;
        // Match the visible viewport's clamped native frame, not the async CDP resize request.
        const x=Math.max(m.position.x/scale,Math.min((m.position.x+m.size.width*q.native.dockRatio)/scale-innerWidth/2,(m.position.x+m.size.width)/scale-innerWidth));
        return {x:x+r.x,y:r.y,w:r.width,h:r.height,opacity:s.opacity,clip:s.clipPath};
      };
      const sample=()=>{
        const live=document.querySelector('main'), content=live.querySelector('[data-dock-frame]');
        const visual=live;
        frames.push({timestamp:(performance.timeOrigin+performance.now())/1000,t:performance.now()-started, viewport:[innerWidth,innerHeight], progress:Number(getComputedStyle(content).getPropertyValue('--dock-progress')),
          morphing:content.hasAttribute('data-morphing'), background:read(visual.querySelector('.notch-backdrop')), avatar:read(visual.querySelector('.pet-avatar')),title:read(visual.querySelector('.latest-title'))});
      };
      sample();
      const live=document.querySelector('main');
      live.querySelector('.notch-panel').dispatchEvent(new MouseEvent('mouseenter'));
      if(live.querySelector('.notch-summary').getAttribute('aria-expanded')!==String(expanded)) live.querySelector('.notch-summary').click();
      if(reverse)setTimeout(()=>live.querySelector('.notch-summary')?.click(),reverse);
      await new Promise(resolve=>{const frame=()=>{sample();if(performance.now()-started<650)requestAnimationFrame(frame);else resolve();};requestAnimationFrame(frame);});
      return frames;
    },{expanded,reverse});
    await client.send('Page.stopScreencast'); recording='';
    await writeFile(resolve(output,`${name}.json`),JSON.stringify(frames,null,2));
    const deltas=frames.slice(1).map((f,i)=>({t:f.t,dx:Math.abs(f.avatar.x-frames[i].avatar.x),dt:f.t-frames[i].t}));
    const maxStep=Math.max(...deltas.map(d=>d.dx));
    const maxFrameStep=Math.max(...deltas.map(d=>d.dx * Math.min(1, 16.67 / d.dt)));
    const widths=frames.map(f=>f.avatar.w);
    results.push({name,frames:frames.length,maxStep,maxFrameStep,avatarSizeSpread:Math.max(...widths)-Math.min(...widths),deltas,first:frames[0],last:frames.at(-1)});
    console.log(name,JSON.stringify({frames:frames.length,maxStep,maxFrameStep,avatarSizeSpread:Math.max(...widths)-Math.min(...widths)}));
  }
  // Deliver a real gateway update while the production component is expanding.
  // The list and header must survive, and its new natural height must be measured.
  await page.evaluate(() => {
    const q = window.__petQa;
    window.__stableDockNodes = ['.notch-summary','.notch-details','.latest-title','.pet-avatar'].map(s => document.querySelector(s));
    q.beforeSnapshot = async () => {
      q.beforeSnapshot = null;
      document.querySelector('.notch-summary').click();
      await new Promise(resolve => setTimeout(resolve, 80));
      window.__updatedDuringMotion = !!document.querySelector('[data-morphing]');
      q.tasks = Array.from({length: 8}, (_, i) => ({id: `updated-${i}`, title: `动画期间更新任务 ${i}`, summary: '新增内容第一行\n新增内容第二行', status: 'running', providerId: 'codex', updatedAt: Date.now() - i}));
    };
  });
  await page.waitForFunction(() => document.querySelector('.latest-title')?.textContent.includes('动画期间'));
  await page.waitForFunction(() => !document.querySelector('[data-morphing]') && innerHeight > 300);
  assert.equal(await page.evaluate(() => window.__updatedDuringMotion), true);
  assert.equal(await page.evaluate(() => window.__stableDockNodes.every(node => node.isConnected)), true);
  assert.equal(await page.locator('.notch-task').count(), 8);
  assert.ok(await page.locator('.notch-task-list').evaluate(node => node.scrollHeight > node.clientHeight));
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({path: resolve(output, 'content-updated.png')});
  await page.locator('.notch-summary').evaluate(node => node.click());
  await page.waitForFunction(() => document.querySelector('[data-dock-frame]')?.getAttribute('data-phase') === 'closed');
  assert.equal(await page.evaluate(() => window.__stableDockNodes.every(node => node.isConnected)), true);
  assert.equal(await page.locator('.dock-body-clip').getAttribute('inert'), '');
  await writeFile(resolve(output,'report.json'),JSON.stringify({results,screenshots,contentUpdate: 'passed',errors},null,2));
  assert.deepEqual(errors,[]);
  for(const r of results){
    assert.ok(r.maxFrameStep<45,`${r.name}: discontinuous screen-space movement ${r.maxStep}px`);
    assert.ok(r.avatarSizeSpread<.1,`${r.name}: avatar was scaled`);
    assert.equal(r.last.morphing,false,`${r.name}: unfinished transition`);
  }
  console.log('Frame-by-frame motion QA passed.');
} finally { await browser?.close(); await new Promise(resolve=>server.httpServer.close(resolve)); }
