# Run after pet_motion_ui_qa.mjs: uv run --with pillow python scripts/pet_motion_images_qa.py
# Pixel check uses the fixture's built-in orange cat, independent of DOM coordinates.
from PIL import Image
import json,pathlib
p=pathlib.Path(__file__).resolve().parent.parent/'artifacts/pet-motion-qa';j=json.loads((p/'report.json').read_text());out=[]
for scene in j['results']:
 name=scene['name']; points=[]
 for s in [s for s in j['screenshots'] if s['name'].rsplit('-',1)[0]==name]:
  im=Image.open(p/s['name']).convert('RGB');a=im.getpixel((0,0));w=a[0]*256+a[1]
  origin=0 if name.startswith('left') else 1440-w if name.startswith('right') else (1440-w)/2
  pixels=[(x,y) for y in range(45) for x in range(w) if (lambda c:c[0]>150 and c[1]>65 and c[0]>c[1]*1.15 and c[2]<70)(im.getpixel((x,y)))]
  assert pixels, f'{name}: missing avatar at {s["name"]}'
  cx=(min(x for x,y in pixels)+max(x for x,y in pixels))/2+origin
  points.append({'x':cx,'timestamp':s['metadata']['timestamp']})
 steps=[abs(b['x']-a['x'])*min(1,.01667/(b['timestamp']-a['timestamp'])) for a,b in zip(points,points[1:]) if b['timestamp']>a['timestamp']]
 maximum=max(steps,default=0);assert maximum<45,(name,maximum)
 out.append({'scenario':name,'screenshots':len(points),'maxPixelStepAt60Hz':maximum,'points':points})
(p/'pixel-report.json').write_text(json.dumps(out,indent=2));print([(s['scenario'],s['screenshots'],round(s['maxPixelStepAt60Hz'],2)) for s in out])

from PIL import Image,ImageDraw
import json,math,pathlib
root=pathlib.Path(__file__).resolve().parent.parent/'artifacts/pet-motion-qa'
report=json.loads((root/'report.json').read_text());gallery=[];animation=[]
for result in report['results']:
 name=result['name']; frames=json.loads((root/f'{name}.json').read_text()); shots=[s for s in report['screenshots'] if s['name'].rsplit('-',1)[0]==name]
 panels=[]
 for index,shot in enumerate(shots):
  t=shot['metadata']['timestamp']; f=min(frames,key=lambda f:abs(f['timestamp']-t)); raw=Image.open(root/shot['name']).convert('RGB'); a=raw.getpixel((0,0));b=raw.getpixel((1,0));w,h=a[0]*256+a[1],b[0]*256+b[1]
  raw=raw.crop((0,0,w,h));raw.putpixel((0,0),(255,255,255));raw.putpixel((1,0),(255,255,255))
  x=30 if name.startswith('left') else 470-w if name.startswith('right') else (500-w)//2
  panel=Image.new('RGB',(500,174),'#e5e7eb');panel.paste(raw,(int(x),24)); d=ImageDraw.Draw(panel)
  d.text((10,6),f'{name}  frame {index:02}  {max(0,round((t-frames[0]["timestamp"])*1000))} ms',fill='#27313f')
  panels.append(panel); path=f'aligned-{name}-{index:03}.png';panel.save(root/path)
  gallery.append({'scenario':name,'frame':index,'ms':max(0,round((t-frames[0]['timestamp'])*1000)),'image':path})
  if name in ('center-open','center-close'): animation.append(panel)
 sheet=Image.new('RGB',(1500,174*math.ceil(len(panels)/3)),'white')
 for i,p in enumerate(panels):sheet.paste(p,((i%3)*500,(i//3)*174))
 sheet.save(root/f'{name}-all-frames.png')
 # Show a readable sample while retaining every frame in the interactive viewer.
 selected=[panels[round(i*(len(panels)-1)/5)] for i in range(6)]
 detail=Image.new('RGB',(1000,522),'white')
 for i,p in enumerate(selected):detail.paste(p,((i%2)*500,(i//2)*174))
 detail.save(root/f'{name}-comparison.png')
animation[0].save(root/'expand-collapse.gif',save_all=True,append_images=animation[1:],duration=40,loop=0)
html='''<!doctype html><html lang="zh"><meta charset="utf-8"><title>Code Pet 逐帧验收</title><style>body{background:#20242b;color:#e5e7eb;font:16px system-ui;margin:32px auto;max-width:900px}img{display:block;width:100%;image-rendering:auto}input{width:100%}button,select{font:inherit;margin:8px;padding:8px}p{color:#b8c2cf}</style><h1>顶部展开 / 收起逐帧对比</h1><p>真实组件的连续截图，按照窗口屏幕坐标对齐。左右箭头逐帧查看。示例任务用于隔离测试，不是真实任务记录。</p><select id="scene"></select><button id="previous">上一帧</button><button id="next">下一帧</button><output id="label"></output><img id="picture"><input id="slider" type="range" min="0"><script>const frames=DATA;const scene=document.querySelector('#scene'),slider=document.querySelector('#slider');for(const name of new Set(frames.map(f=>f.scenario)))scene.add(new Option(name,name));let list=[];function draw(){const f=list[+slider.value];picture.src=f.image;label.textContent=`第 ${f.frame+1}/${list.length} 帧 · ${f.ms}ms`}function change(){list=frames.filter(f=>f.scenario===scene.value);slider.max=list.length-1;slider.value=0;draw()}function step(n){slider.value=Math.max(0,Math.min(list.length-1,+slider.value+n));draw()}scene.onchange=change;slider.oninput=draw;previous.onclick=()=>step(-1);next.onclick=()=>step(1);onkeydown=e=>{if(e.key==='ArrowLeft'){e.preventDefault();step(-1)}if(e.key==='ArrowRight'){e.preventDefault();step(1)}};change()</script></html>'''.replace('DATA',json.dumps(gallery))
(root/'index.html').write_text(html)
print(len(gallery),'screenshots aligned; all-frame sheets, GIF and viewer saved')
