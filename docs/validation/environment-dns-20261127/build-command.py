from pathlib import Path
import subprocess,os
r=Path('/Users/main/dev/trames-mac')
subprocess.run(['cargo','build','-p','trames-mac','--bin','trames-mac-run','--locked'],cwd=r,check=True)
b=r/'target/debug/trames-mac-run'
subprocess.run(['codesign','--force','--sign','-','--entitlements',r/'apps/macos/Trames.entitlements',b],check=True)
subprocess.run(['codesign','--verify','--strict',b],check=True)
