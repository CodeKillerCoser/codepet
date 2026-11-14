// Production Plane Cards, sampled at the deterministic browser clock's 16ms rAF cadence.
import { resolve } from 'node:path';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { build, preview } from 'vite';
const output = resolve('artifacts/pet-plane-stack-qa'), outDir = resolve(output, 'site');
await mkdir(output, { recursive: true });
await rm(resolve(output, 'report.json'), { force: true });
const frameMs = 16;
await build({ build: { outDir, emptyOutDir: false, rollupOptions: { input: resolve('frontend/qa/pet-module.html') } } });
const server = await preview({ build: { outDir }, preview: { host: '127.0.0.1', port: 1437, strictPort: true } });
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 640, height: 440 }, deviceScaleFactor: 2 });
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.exposeFunction('__resizeNativeWindow', size => page.setViewportSize(size));
  await page.goto('http://127.0.0.1:1437/frontend/qa/pet-module.html?frame');
  await page.waitForSelector('.pet-avatar img');
  await page.evaluate(() => {
    const q = window.__petQa, now = Date.now();
    q.settings.pet.selectedPetId = 'builtin:panda'; q.settings.pet.kind = 'builtin'; q.updateSettings();
    q.tasks = [
      { id:'live', title:'检查任务面板交互', summary:'进行中的任务保持在顶部', providerId:'codex', status:'running', updatedAt: now },
      ...['窗口布局检查', '自定义角色导入', '远程连接回归', '任务状态同步', '资源加载验证', '列表键盘导航'].map((title, i) => ({
        id:`plane-${i}`, title, summary:['布局与尺寸检查通过\n结果已保存，可以查看详情', '角色头像和动画状态检查通过\n保留现有导入能力', '多设备连接检查通过\n等待下一轮验证'][i % 3], providerId:'codex', status:i === 2 ? 'interrupted' : 'completed', updatedAt:now - (i + 1) * 60000,
      })),
    ];
    q.surface.availableWidth = 2560; q.surface.availableHeight = 1440; q.finishDrag(true);
  });
  await page.waitForSelector('.notch-header');
  await page.locator('.notch-header').hover();
  await page.waitForFunction(() => document.querySelector('[data-dock-frame]')?.dataset.phase === 'open' && !document.querySelector('[data-dock-frame]')?.hasAttribute('data-morphing'));
  const stack = page.locator('#notch-completed-codex');
  await stack.locator('.notch-task:not([inert])').hover();
  await page.waitForTimeout(400);
  // Reserve a fixed preview canvas so expanded list scrolling is visible too.
  // No app CSS or motion timings are replaced for this capture.
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  const wheel = delta => stack.dispatchEvent('wheel', { deltaY: delta, bubbles: true, cancelable: true });
  const read = () => page.evaluate(() => {
    const group = document.querySelector('#notch-completed-codex');
    const box = el => { const r = el.getBoundingClientRect(); return { x:r.x, y:r.y, width:r.width, height:r.height }; };
    return { now:performance.now(), position: Number(group.dataset.planePosition), moving:group.hasAttribute('data-plane-moving'), stage:box(group),
      header:box(document.querySelector('.notch-header')), scroll:document.querySelector('.notch-task-list').scrollTop,
      cards:[...group.querySelectorAll('.notch-task')].map(el => {
        const content = el.querySelector('.task-content'), css = getComputedStyle(content), matrix = new DOMMatrixReadOnly(css.transform);
        return { id:el.dataset.taskId, ...box(content), a:matrix.a, b:matrix.b, c:matrix.c, d:matrix.d,
          ink:Number(getComputedStyle(content.querySelector('.task-topline')).opacity), opacity:Number(getComputedStyle(el).opacity), front:!el.hasAttribute('inert'), layer:Number(getComputedStyle(el).zIndex) };
      }) };
  });
  const scenes = [];
  async function capture(name, events, length = 52) {
    const frames = [];
    for (let i = 0; i < length; i++) {
      if (events[i]) await events[i]();
      await page.clock.runFor(frameMs);
      const frame = await read(); frame.file = `${name}-${String(i).padStart(3,'0')}.png`; frame.ms = i * frameMs;
      await page.screenshot({ path: resolve(output, frame.file), animations:'allow' });
      frames.push(frame);
    }
    await writeFile(resolve(output, `${name}.json`), JSON.stringify(frames, null, 2));
    // Every captured frame: fixed plane orientation, constant stage/header,
    // continuous translation/scale/ink, exactly one accessible foreground card.
    for (let i = 0; i < frames.length; i++) {
      const frame = frames[i];
      assert.equal(frame.cards.filter(card => card.front).length, 1);
      assert.equal(frame.stage.height, frames[0].stage.height, `${name}: stack resized during orbit`);
      assert.equal(frame.stage.width, frames[0].stage.width, `${name}: stack width changed`);
      assert.deepEqual(frame.header, frames[0].header, `${name}: header moved`);
      assert.equal(frame.scroll, frames[0].scroll, `${name}: wheel scrolled the list`);
      if (!frame.moving) {
        const front = frame.cards.find(card => card.front);
        assert.ok(Math.abs(front.y - frame.stage.y) < 1, `${name}: blank space above compact stack`);
        for (const card of frame.cards) {
          assert.ok(card.y >= front.y - .1, `${name}: rear card exposed above at rest`);
          assert.ok(card.y + card.height <= front.y + front.height + 22.1, `${name}: oversized rear card at rest`);
        }
      }
      for (const card of frame.cards) {
        assert.equal(card.b, 0); assert.equal(card.c, 0); assert.equal(card.a, card.d);
        assert.ok(card.a >= 600 / 636 - .000001 && card.a <= 1, `${name}: card width escaped the original stack range`);
        if (!i) continue;
        const elapsed = Math.max(1, (frame.now - frames[i - 1].now) / frameMs);
        const previous = frames[i - 1].cards.find(old => old.id === card.id);
        if (!previous) { assert.ok(card.opacity < .25, `${name}: card popped in`); continue; }
        assert.ok(Math.abs((card.y + card.height / 2) - (previous.y + previous.height / 2)) < 7 * elapsed, `${name}: vertical jump at ${i}: ${card.id}, ${card.y + card.height / 2} vs ${previous.y + previous.height / 2}`);
        assert.ok(Math.abs(card.a - previous.a) < .045 * elapsed, `${name}: scale jump`);
        assert.ok(Math.abs(card.ink - previous.ink) < .26 * elapsed, `${name}: text flash`);
      }
    }
    assert.equal(frames.at(-1).moving, false, `${name}: did not settle`);
    scenes.push({ name, frames });
    console.log(`${name}: ${frames.length} frames verified`);
  }
  await capture('forward', { 8:() => wheel(100) });
  assert.equal((await read()).position, 1);
  await capture('next', { 8:() => wheel(100) });
  assert.equal((await read()).position, 2);
  await capture('reverse', { 8:() => wheel(-100) });
  assert.equal((await read()).position, 1);
  await capture('retarget', { 4:() => wheel(100), 16:() => wheel(-100) }, 68);
  assert.equal((await read()).position, 1);
  await wheel(-100); await page.clock.runFor(1000);
  assert.equal((await read()).position, 0);
  await capture('wrap', { 8:() => wheel(-100) });
  assert.equal((await read()).position, 5);
  await page.clock.resume();
  await page.evaluate(() => {
    window.__planeFixture = window.__petQa.tasks;
    window.__petQa.tasks = window.__planeFixture.slice(0, 3);
  });
  await page.waitForFunction(() => document.querySelector('.completed-count')?.textContent === '2');
  await page.waitForTimeout(350);
  await page.clock.pauseAt(new Date(await page.evaluate(() => Date.now() + 50)));
  await capture('two-cards', { 8:() => wheel(100) }, 68);
  assert.equal((await read()).cards.length, 2);
  assert.equal((await read()).position, 1);
  await page.clock.resume();
  await page.evaluate(() => { window.__petQa.tasks = window.__planeFixture; });
  await page.waitForFunction(() => document.querySelector('.completed-count')?.textContent === '6');
  await page.waitForTimeout(350);
  // Expanded mode removes all transforms and keeps native scrolling.
  await page.locator('.completed-toggle').click();
  await page.waitForTimeout(800);
  assert.equal(await stack.locator('.notch-task').count(), 6);
  assert.equal(await stack.locator('[inert]').count(), 0);
  assert.ok(await stack.locator('.task-content').evaluateAll(nodes => nodes.every(node => getComputedStyle(node).transform === 'none')));
  await page.screenshot({ path: resolve(output, 'expanded.png') });
  await page.mouse.move(300, 180);
  await page.mouse.wheel(0, 180);
  await page.waitForFunction(() => document.querySelector('.notch-task-list').scrollTop > 0);
  assert.ok(await page.locator('.notch-task-list').evaluate(el => el.scrollTop > 0));
  await page.screenshot({ path: resolve(output, 'expanded-scroll.png') });
  // Reduced motion snaps directly and does not leave a running animation.
  await page.locator('.completed-toggle').click(); await page.waitForTimeout(800);
  await page.emulateMedia({ reducedMotion:'reduce' }); await page.waitForTimeout(50);
  await wheel(100); await page.waitForTimeout(30);
  assert.equal((await read()).moving, false);
  assert.equal((await read()).position, 2);
  assert.deepEqual(errors, []);
  await writeFile(resolve(output, 'report.json'), JSON.stringify({ scenes, expandedScroll:true, reducedMotion:true }, null, 2));
  console.log('Plane stack QA passed: orbit/reverse/retarget, fixed planes, stable layout, expanded scrolling, reduced motion.');
} finally { await browser?.close(); await new Promise(done => server.httpServer.close(done)); }
