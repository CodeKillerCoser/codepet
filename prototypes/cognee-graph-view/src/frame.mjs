window.addEventListener('message', function receive(event) {
  if (event.source !== parent || event.origin !== location.origin) return;
  if (event.data?.type !== 'codepet:cognee:render' || typeof event.data.html !== 'string') return;
  window.removeEventListener('message', receive);
  // Retain frame.html's real URL so upstream history/URL-state controls still work.
  const ready = `<script>
    const initialized = window.d3 && ['_requestGraphRedraw','_renderSchemaGraph','_renderMemoryView','_renderSemanticView'].every(key => typeof window[key] === 'function');
    parent.postMessage(initialized
      ? {type:'codepet:cognee:ready',nodes:Object.keys(window._vizNodeById||{}).length,links:(window._vizLinks||[]).length}
      : {type:'codepet:cognee:error',message:'Original Cognee views did not initialize; check local assets'}, location.origin);
  <\/script>`;
  const html = event.data.html.replace('</body>', () => ready + '</body>');
  document.open();
  document.write(html);
  document.close();
});
