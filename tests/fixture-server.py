"""Local media fixture for desktop smoke testing; no third-party website required."""
import http.server
import pathlib
import platform
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "work" / "fixtures"
FIXTURES.mkdir(parents=True, exist_ok=True)
SAMPLE = FIXTURES / "sample.mp4"
if not SAMPLE.exists():
    target = "win-x64" if platform.system() == "Windows" else "mac-arm64" if platform.machine() == "arm64" else "mac-x64"
    binary = "ffmpeg.exe" if target == "win-x64" else "ffmpeg"
    subprocess.run([str(ROOT / "vendor" / target / binary), "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24", "-f", "lavfi", "-i", "sine=frequency=440", "-t", "6", "-c:v", "mpeg4", "-c:a", "aac", str(SAMPLE)], check=True)


class Handler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/slow.mp4":
            self.send_response(200)
            self.send_header("Content-Type", "video/mp4")
            self.send_header("Content-Length", str(SAMPLE.stat().st_size))
            self.end_headers()
            try:
                with SAMPLE.open("rb") as source:
                    while block := source.read(4096):
                        self.wfile.write(block)
                        self.wfile.flush()
                        time.sleep(0.1)
            except (BrokenPipeError, ConnectionResetError):
                pass
        else:
            super().do_GET()

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(FIXTURES), **kwargs)


print("Fixture server: http://127.0.0.1:18767/sample.mp4 or /slow.mp4", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", 18767), Handler).serve_forever()
