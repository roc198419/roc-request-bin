"""Generate native icon containers from the same geometry as assets/app.svg.
Requires Pillow. Run only when changing the brand icon; assets are committed.
"""
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parents[1] / "assets"
im = Image.new("RGBA", (1024, 1024))
d = ImageDraw.Draw(im)
d.rounded_rectangle((48, 48, 976, 976), radius=208, fill="#2454ce")
d.rounded_rectangle((256, 538, 768, 790), radius=40, fill="white")
d.polygon([(256,538),(384,538),(448,634),(576,634),(640,538),(768,538),(768,520),(256,520)],fill="#2454ce")
d.line([(512,246),(512,516)],fill="white",width=60)
d.line([(410,418),(512,520),(614,418)],fill="white",width=60,joint="curve")
for x,y in [(512,246),(410,418),(614,418),(512,520)]:
    d.ellipse((x-30,y-30,x+30,y+30), fill="white")
im.save(root / "app.png")
im.save(root / "app.ico",sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
im.save(root / "app.icns",sizes=[(128,128),(256,256),(512,512),(1024,1024)])
