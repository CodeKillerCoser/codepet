window.addEventListener('message',function receive(event) {
  if (event.source !== parent || event.data?.type !== 'codepet:cognee:render'
      || typeof event.data.html !== 'string' || typeof event.data.token !== 'string') return;
  window.removeEventListener('message',receive);
  const token = JSON.stringify(event.data.token).replace(/</g,'\\u003c');
  // Original view state stays inside this sandbox, separate from App navigation.
  const boundary = `<script>
    const replaceState = history.replaceState.bind(history);
    history.replaceState = (...args) => {try {replaceState(...args)} catch(error) {if(error.name !== 'SecurityError') throw error}};
  <\/script>`;
  const ready = `<script>
    const initialized = window.d3 && ['_requestGraphRedraw','_renderSchemaGraph','_renderMemoryView','_renderSemanticView'].every(key => typeof window[key] === 'function');
    if (initialized) document.getElementById('btn-fit')?.click();
    parent.postMessage({type:initialized?'codepet:cognee:ready':'codepet:cognee:error',token:${token},
      nodes:Object.keys(window._vizNodeById||{}).length,links:(window._vizLinks||[]).length},'*');
  <\/script>`;
  const html = event.data.html.replace('<head>',() => '<head>'+boundary).replace('</body>',() => ready+'</body>');
  document.open(); document.write(html); document.close();
});
