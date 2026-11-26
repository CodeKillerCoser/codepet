// Integration hooks around unmodified upstream views. Semantic data stays native.
function connectViewState(token, initial) {
  if (typeof document.querySelector !== 'function') return;
  let interacting = false, timer;
  function report() {
    clearTimeout(timer);
    const graph = {scale:window.scale,x:window.tx,y:window.ty};
    parent.postMessage({type:'codepet:cognee:state',token,state:{
      view:document.querySelector('.tab-btn.active')?.dataset.view ?? 'graph',
      light:!!window._isLightMode,search:document.getElementById('search-input')?.value ?? '',
      graph,interacting}},'*');
  }
  function later() { clearTimeout(timer); timer = setTimeout(report,100); }
  document.addEventListener('pointerdown',() => { interacting = true; report(); });
  for (const type of ['pointerup','pointercancel']) document.addEventListener(type,() => { interacting = false; later(); });
  for (const type of ['click','input','change','wheel']) document.addEventListener(type,later);
  initial = initial ?? {};
  if (typeof initial.light === 'boolean' && initial.light !== !!window._isLightMode) document.getElementById('theme-toggle')?.click();
  if (typeof initial.search === 'string') {
    const input = document.getElementById('search-input');
    if (input) { input.value = initial.search; input.dispatchEvent(new Event('input',{bubbles:true})); }
  }
  if (['graph','schema','memory','semantic'].includes(initial.view)) document.querySelector(`.tab-btn[data-view="${initial.view}"]`)?.click();
  const graph = initial.graph;
  if (graph && [graph.scale,graph.x,graph.y].every(Number.isFinite) && graph.scale > 0 && graph.scale <= 100) {
    window.zoomAnim = null;
    window.scale = graph.scale; window.tx = graph.x; window.ty = graph.y;
    window._requestGraphRedraw?.();
  }
  report();
}

function connectSemantic(token) {
  const nativeRender = window._renderSemanticView;
  let loaded = !!window._semanticPositions, pending = false;
  window._renderSemanticView = function (...args) {
    if (loaded) return nativeRender(...args);
    document.getElementById('semantic-status').textContent = '正在读取语义布局…';
    if (!pending) {
      pending = true;
      parent.postMessage({type:'codepet:cognee:semantic',token},'*');
    }
  };
  window.addEventListener('message',event => {
    if (event.source !== parent || event.data?.token !== token) return;
    if (event.data.type === 'codepet:cognee:semantic-error') {
      pending = false;
      document.getElementById('semantic-status').textContent = event.data.retryable
        ? '语义布局正在后台准备，稍后自动更新…'
        : '语义布局暂不可用，请再次点击 Semantic 重试；图谱更新后请刷新页面。';
      if (event.data.retryable) setTimeout(() => {
        if (!loaded && !pending && document.getElementById('semantic-view').style.display !== 'none') window._renderSemanticView();
      },2000);
    } else if (event.data.type === 'codepet:cognee:semantic-result') {
      window._semanticPositions = event.data.positions;
      const clusters = window._codepetSemanticClusters;
      Object.keys(clusters).forEach(key => delete clusters[key]);
      Object.assign(clusters,event.data.clusters ?? {});
      pending = false; loaded = true;
      if (document.getElementById('semantic-view').style.display !== 'none') nativeRender();
    }
  });
}

window.addEventListener('message',function receive(event) {
  if (event.source !== parent || event.data?.type !== 'codepet:cognee:render'
      || typeof event.data.html !== 'string' || typeof event.data.token !== 'string') return;
  window.removeEventListener('message',receive);
  const token = JSON.stringify(event.data.token).replace(/</g,'\\u003c');
  const state = JSON.stringify(event.data.viewState ?? {}).replace(/</g,'\\u003c');
  // Original view state stays inside this sandbox, separate from App navigation.
  const boundary = `<script>
    const replaceState = history.replaceState.bind(history);
    history.replaceState = (...args) => {try {replaceState(...args)} catch(error) {if(error.name !== 'SecurityError') throw error}};
  <\/script>`;
  const ready = `<script>
    const initialized = window.d3 && ['_requestGraphRedraw','_renderSchemaGraph','_renderMemoryView','_renderSemanticView'].every(key => typeof window[key] === 'function');
    if (initialized && ${event.data.lazySemantic === true}) (${connectSemantic.toString()})(${token});
    // Story nodes already have fixed native targets. Do not wait several seconds
    // for a force simulation whose ticks cannot move any of them.
    const pinned = Object.values(window._vizNodeById || {});
    if (initialized && pinned.length && pinned.every(node => Number.isFinite(node.fx) && Number.isFinite(node.fy))) {
      pinned.forEach(node => { node.x = node.fx; node.y = node.fy; });
      window._requestGraphRedraw();
      document.getElementById('loading-overlay')?.classList.add('hidden');
    }
    if (initialized) document.getElementById('btn-fit')?.click();
    if (initialized) (${connectViewState.toString()})(${token},${state});
    parent.postMessage({type:initialized?'codepet:cognee:ready':'codepet:cognee:error',token:${token},
      nodes:Object.keys(window._vizNodeById||{}).length,links:(window._vizLinks||[]).length},'*');
  <\/script>`;
  const html = event.data.html.replace('<head>',() => '<head>'+boundary).replace('</body>',() => ready+'</body>');
  document.open(); document.write(html); document.close();
});
