"""Encode the project icon at Windows icon sizes (requires Pillow)."""
from pathlib import Path
from PIL import Image
root = Path(__file__).resolve().parents[2]
image = Image.open(root / "assets/qing-zip.png").convert("RGBA")
image.save(root / "obz-gui/icons/icon.ico", sizes=[(n,n) for n in (16,24,32,48,64,128,256)])
image.resize((128,128),Image.Resampling.LANCZOS).save(root / "obz-gui/frontend/app-icon.png")
