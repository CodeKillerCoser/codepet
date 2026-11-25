import { assemble, normalizeSnapshot, viewFiles } from './assemble.mjs';

const resourceCache = new Map();
async function loadResources(url) {
  const key = String(url);
  if (!resourceCache.has(key)) {
    const files = [['template', 'template.html'], ...Object.values(viewFiles).map(name => [name, name + '.js'])];
    const promise = Promise.all(files.map(async ([name, path]) => {
      const response = await fetch(new URL(path, url));
      if (!response.ok) throw new Error(`Cannot load Cognee asset ${path}: ${response.status}`);
      return [name, await response.text()];
    })).then(Object.fromEntries);
    resourceCache.set(key, promise);
    promise.catch(() => resourceCache.delete(key));
  }
  return resourceCache.get(key);
}

/** Mount original Cognee views in an isolated document; await update() calls in order. */
export async function mountCogneeGraph(host, input, options = {}) {
  if (!(host instanceof HTMLElement)) throw new TypeError('An HTMLElement host is required');
  normalizeSnapshot(input);
  const assetsURL = new URL(options.assetsURL ?? '../assets/', import.meta.url);
  const frameURL = new URL('../frame.html', assetsURL);
  if (frameURL.origin !== location.origin) throw new Error('Host and renderer assets must use the same origin');
  const resources = await loadResources(assetsURL);
  let current = null, disposed = false, pending = false, pendingFrame = null, cancelPending = null;

  async function update(snapshot) {
    if (disposed) throw new Error('Renderer has been destroyed');
    if (pending) throw new Error('Await the previous update before submitting another snapshot');
    const html = assemble(snapshot, resources, assetsURL);
    const frame = document.createElement('iframe');
    frame.title = options.title ?? 'Cognee graph visualization';
    frame.style.cssText = 'display:block;width:100%;height:100%;border:0;';
    frame.src = frameURL.href;
    pending = true;
    pendingFrame = frame;
    try {
      const ready = new Promise((resolve, reject) => {
        const timeout = setTimeout(() => finish(new Error('Cognee frame initialization timed out')), 15000);
        function finish(error, detail) {
          clearTimeout(timeout);
          window.removeEventListener('message', onMessage);
          error ? reject(error) : resolve(detail);
        }
        cancelPending = () => finish(new Error('Renderer was destroyed while loading'));
        function onMessage(event) {
          if (event.source !== frame.contentWindow || event.origin !== location.origin) return;
          if (event.data?.type === 'codepet:cognee:ready') finish(null, event.data);
          if (event.data?.type === 'codepet:cognee:error') finish(new Error(event.data.message));
        }
        window.addEventListener('message', onMessage);
        frame.addEventListener('load', () => {
          frame.contentWindow.postMessage({ type: 'codepet:cognee:render', html }, location.origin);
        }, { once: true });
        frame.addEventListener('error', () => finish(new Error('Cannot load renderer frame')), { once: true });
      });
      host.append(frame);
      const detail = await ready;
      if (disposed) throw new Error('Renderer was destroyed while loading');
      current?.remove();
      current = frame;
      return detail;
    } catch (error) { frame.remove(); throw error; }
    finally { pending = false; pendingFrame = null; cancelPending = null; }
  }

  const detail = await update(input);
  return {
    detail,
    update,
    destroy() {
      disposed = true;
      current?.remove();
      pendingFrame?.remove();
      cancelPending?.();
      current = null;
    },
  };
}
