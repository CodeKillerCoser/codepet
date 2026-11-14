"""Assemble the captured browser frames, without synthesizing animation frames.
Run after test:pet:stack: uv run --with pillow python scripts/pet_plane_stack_preview.py
"""
import json
import math
from pathlib import Path
from PIL import Image, ImageDraw, ImageChops

root = Path(__file__).resolve().parent.parent / 'artifacts/pet-plane-stack-qa'
report = json.loads((root / 'report.json').read_text())
preview = []
durations = []
for scene in report['scenes']:
    frames = scene['frames']
    first = Image.open(root / frames[0]['file']).convert('RGB')
    # Check the actual pixels as well as the DOM: the fixed top bar must stay put.
    # Exclude the pulsing running-status dot and the time label outside the bar.
    # The browser capture uses deviceScaleFactor=2.
    pixel_scale = 2
    header = frames[0]['header']
    region = (round((header['x'] + 90) * pixel_scale), round(header['y'] * pixel_scale),
              round((header['x'] + header['width']) * pixel_scale), round((header['y'] + header['height']) * pixel_scale))
    reference = first.crop(region)
    tiles = []
    for i, frame in enumerate(frames):
        image = Image.open(root / frame['file']).convert('RGB')
        assert image.size == first.size, f'{scene["name"]}: screenshot size changed'
        assert ImageChops.difference(reference, image.crop(region)).getbbox() is None, f'{scene["name"]}: header pixels changed at frame {i}'
        thumb = image.copy()
        thumb.thumbnail((350, 210))
        tile = Image.new('RGB', (350, 234), '#e5e7eb')
        tile.paste(thumb, (0, 24))
        ImageDraw.Draw(tile).text((8, 6), f'{scene["name"]} | {i:02} | {round(frame["ms"])} ms', fill='#15202d')
        tiles.append(tile)
        if scene['name'] in ('next', 'reverse', 'retarget'):
            image.thumbnail((880, 600))
            preview.append(image)
            # GIF stores centiseconds: this pattern preserves the 16ms frame cadence.
            durations.append(450 if i == len(frames) - 1 else (20, 10, 20, 10, 20)[i % 5])
    sheet = Image.new('RGB', (350 * 6, 234 * math.ceil(len(tiles) / 6)), '#e5e7eb')
    for i, tile in enumerate(tiles): sheet.paste(tile, ((i % 6) * 350, (i // 6) * 234))
    sheet.save(root / f'{scene["name"]}-all-frames.jpg', quality=90)
preview[0].save(root / 'plane-cards-preview.gif', save_all=True, append_images=preview[1:], duration=durations, loop=0, optimize=False, disposal=2)
scenes = json.dumps([{ 'name': s['name'], 'files': [f['file'] for f in s['frames']] } for s in report['scenes']])
(root / 'frames.html').write_text('''<!doctype html><meta charset="utf-8"><title>Plane Cards · 逐帧验收</title>
<style>body{margin:32px;background:#e8eaed;color:#20242c;font:16px system-ui}main{max-width:900px;margin:auto}nav{display:flex;gap:12px;align-items:center;margin:20px 0}img{max-width:100%;background:black;border-radius:12px}input{flex:1}button,select{padding:8px}p{line-height:1.6}</style>
<main><h1>Plane Cards · 逐帧验收</h1><p>真实组件与示例任务；每帧间隔 16 毫秒。卡片沿纵向轨道循环，平面不旋转。可拖动进度条，或按左右方向键逐帧对比。</p>
<nav><select id="scene"></select><button id="previous">上一帧</button><input id="position" type="range" min="0" value="0"><button id="next">下一帧</button><span id="label"></span></nav><img id="frame"><p><a href="plane-cards-preview.gif">循环动效</a> · <a href="expanded.png">展开列表</a> · <a href="expanded-scroll.png">展开后滚动</a> · <a href="report.json">检查数据</a></p></main>
<script>const scenes=''' + scenes + ''';const scene=document.querySelector('#scene'),position=document.querySelector('#position');for(const [i,s]of scenes.entries())scene.add(new Option(s.name,i));function show(){const s=scenes[+scene.value],i=Math.min(+position.value,s.files.length-1);position.max=s.files.length-1;position.value=i;document.querySelector('#frame').src=s.files[i];document.querySelector('#label').textContent=`${i+1}/${s.files.length}`}scene.onchange=()=>{position.value=0;show()};position.oninput=show;function step(n){position.value=Math.max(0,Math.min(+position.max,+position.value+n));show()}document.querySelector('#previous').onclick=()=>step(-1);document.querySelector('#next').onclick=()=>step(1);document.onkeydown=e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();step(e.key==='ArrowRight'?1:-1)}};show()</script>''')
print(f'{sum(len(s["frames"]) for s in report["scenes"])} frames checked; GIF and frame viewer saved in {root}')
