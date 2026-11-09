"""Check CUA screenshots of the centered native panda fixture, not DOM samples.

Input: a directory containing cycle.json [{i,t}] and cycle-<i>.png captured
continuously with cua App.getScreenshot. Run the pet_surface_qa --capture
bundle first. Screenshot dimensions must be physical pixels on a 2x display.
This check deliberately fails on the .5 blank/clipped/shifted native frames.
Independent native cover windows are NOT captured by App.getScreenshot: use
desktop recording for final acceptance of that implementation, not this script.
"""
import json
import sys
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(sys.argv[1])
frames = json.loads((root / 'cycle.json').read_text())
results, images, problems = [], [], []
for frame in frames:
    im = Image.open(root / f"cycle-{frame['i']}.png").convert('RGB')
    black = [x for x in range(im.width) if max(im.getpixel((x, 10))) < 20]
    center, avatar = None, 0
    if black:
        center = (min(black) + max(black)) / 2 - im.width / 2
        # Panda lies just inside the left shoulder, before any status text.
        avatar = sum(
            1 for y in range(8, min(im.height, 60))
            for x in range(min(black) + 8, min(im.width, min(black) + 92))
            if (lambda c: min(c) > 130 and max(c)-min(c) < 65)(im.getpixel((x, y)))
        )
    if center is None or abs(center) > 3 or avatar < 80:
        problems.append(frame['i'])
    results.append({**frame, 'size': im.size, 'centerOffset': center, 'avatarPixels': avatar})
    canvas = Image.new('RGB', (1100, 720), '#dddddd')
    canvas.paste(im, ((1100-im.width)//2, 0))
    ImageDraw.Draw(canvas).text((10, 700), f"Native WKWebView | frame {frame['i']} | {frame['t']} ms", fill='black')
    images.append(canvas)

(root / 'pixel-report.json').write_text(json.dumps({'frames': results, 'failures': problems}, indent=2))
durations = [max(20, b['t']-a['t']) for a,b in zip(frames, frames[1:])] + [40]
images[0].save(root / 'native-cycle.gif', save_all=True, append_images=images[1:], duration=durations, loop=0)
# Every changing frame and its immediate neighbors, no cherry-picked transitions.
changes = {i for i in range(1,len(results)) if results[i]['size'] != results[i-1]['size'] or images[i].crop((0,0,1100,170)) != images[i-1].crop((0,0,1100,170))}
indices = sorted({j for i in changes for j in (i-1,i,i+1) if 0 <= j < len(images)})
sheet = Image.new('RGB',(1100, ((len(indices)+1)//2)*105),'#dddddd')
draw = ImageDraw.Draw(sheet)
for slot, i in enumerate(indices):
    x,y=slot%2*550,slot//2*105
    sheet.paste(images[i].crop((0,0,1100,170)).resize((550,85)),(x,y))
    draw.text((x,y+85),f"Frame {i} | {frames[i]['t']} ms",fill='black')
sheet.save(root / 'native-transitions.png')
print(json.dumps({'frames': len(results), 'failedFrames': problems}))
assert not problems, f'Native pixel discontinuity in frames {problems}'
