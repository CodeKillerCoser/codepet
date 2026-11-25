import { assemble, normalizeSnapshot, viewFiles } from './assemble.js';

let cached;
async function resources(base) {
  if (cached) return cached;
  cached = (async () => {
    const entries = [['template','template.html'],['d3','d3.v7.9.0.min.js'],['fontCSS','inter.css'],
      ...Object.values(viewFiles).map(name => [name,name+'.js'])];
    const result = Object.fromEntries(await Promise.all(entries.map(async ([key,path]) => {
      const response = await fetch(new URL('vendor/'+path,base));
      if (!response.ok) throw new Error('无法加载图预览资源：'+path);
      return [key,await response.text()];
    })));
    const embed = await fetch(new URL('embed.css',base));
    if (!embed.ok) throw new Error('无法加载图预览布局');
    result.embedCSS = await embed.text();
    const matches = [...result.fontCSS.matchAll(/url\(([^)]+)\)/g)];
    for (const [match,path] of matches) {
      const response = await fetch(new URL('vendor/'+path.replace(/^\.\//,''),base));
      if (!response.ok) throw new Error('无法加载图预览字体');
      const bytes = new Uint8Array(await response.arrayBuffer());
      let binary = '';
      for (let i=0;i<bytes.length;i+=8192) binary += String.fromCharCode(...bytes.subarray(i,i+8192));
      result.fontCSS = result.fontCSS.replace(match,'url(data:font/ttf;base64,'+btoa(binary)+')');
    }
    return result;
  })();
  cached.catch(() => { cached = undefined; });
  return cached;
}

/** Mount the pinned, unmodified Cognee views; no model or database APIs in this layer. */
export async function mountCogneeGraph(host,input,options={}) {
  normalizeSnapshot(input);
  const base = new URL('./',import.meta.url);
  const html = assemble(input,await resources(base));
  const frame = document.createElement('iframe');
  frame.title = options.title ?? 'Cognee 知识图谱预览';
  frame.setAttribute('sandbox','allow-scripts');
  frame.style.cssText = 'display:block;width:100%;height:100%;border:0;visibility:hidden';
  frame.src = new URL('frame.html',base).href;
  const token = crypto.randomUUID();
  let cleanup;
  const ready = new Promise((resolve,reject) => {
    const timer = setTimeout(() => finish(new Error('图预览初始化超时，请重试')),15000);
    function finish(error,detail) {
      clearTimeout(timer); window.removeEventListener('message',receive);
      error ? reject(error) : resolve(detail);
    }
    function receive(event) {
      if (event.source !== frame.contentWindow || event.origin !== 'null' || event.data?.token !== token) return;
      if (event.data.type === 'codepet:cognee:ready') finish(null,event.data);
      if (event.data.type === 'codepet:cognee:error') finish(new Error('Cognee 图预览初始化失败'));
    }
    cleanup = () => { frame.remove(); finish(new Error('图预览已取消')); };
    window.addEventListener('message',receive);
    // An opaque sandbox cannot receive a message targeted to the Host origin.
    frame.addEventListener('load',() => frame.contentWindow.postMessage({type:'codepet:cognee:render',html,token},'*'),{once:true});
    frame.addEventListener('error',() => finish(new Error('无法打开图预览')),{once:true});
  });
  const abort = () => cleanup();
  if (options.signal?.aborted) { abort(); await ready; }
  options.signal?.addEventListener('abort',abort,{once:true});
  host.append(frame);
  try {
    const detail = await ready;
    frame.style.visibility = 'visible';
    return {detail,destroy(){frame.remove();}};
  } catch (error) { frame.remove(); throw error; }
  finally { options.signal?.removeEventListener('abort',abort); }
}
