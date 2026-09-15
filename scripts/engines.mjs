import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { gunzipSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';

const version = '2026.08.19';
const targets = process.argv.slice(2);
const base = `https://github.com/yt-dlp/yt-dlp/releases/download/${version}`;
async function download(url) {
  return execFileSync('curl', ['--fail', '--location', '--retry', '3', '--connect-timeout', '30', '--max-time', '300', '--silent', '--show-error', url], { maxBuffer: 256 * 1024 * 1024 });
}
const checksums = (await download(`${base}/SHA2-256SUMS`)).toString();
for (const target of targets.length ? targets : ['mac-arm64', 'mac-x64', 'win-x64']) {
  if (!['mac-arm64', 'mac-x64', 'win-x64'].includes(target)) throw new Error('Unknown target');
  const dir = path.join('vendor', target);
  await fs.mkdir(dir, { recursive: true });
  const win = target.startsWith('win');
  const filename = win ? 'yt-dlp.exe' : 'yt-dlp_macos';
  const enginePath = path.join(dir, win ? 'yt-dlp.exe' : 'yt-dlp');
  const engine = await fs.readFile(enginePath).catch(() => download(`${base}/${filename}`));
  const hash = createHash('sha256').update(engine).digest('hex');
  if (!checksums.split('\n').some(line => line.startsWith(hash) && line.endsWith(filename))) throw new Error('yt-dlp checksum mismatch');
  await fs.writeFile(path.join(dir, win ? 'yt-dlp.exe' : 'yt-dlp'), engine, { mode: 0o755 });
  if (win) {
    const ffbase = 'https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1';
    for (const tool of ['ffmpeg', 'ffprobe']) {
      const dest = path.join(dir, tool + '.exe');
      if (!(await fs.stat(dest).catch(() => null))) {
        await fs.writeFile(dest, gunzipSync(await download(`${ffbase}/${tool}-win32-x64.gz`)));
      }
    }
    for (const file of ['LICENSE', 'README']) {
      await fs.writeFile(path.join(dir, `FFmpeg-${file}.txt`), await download(`${ffbase}/win32-x64.${file}`));
    }
  } else {
    if (!(await fs.stat('work/ffmpeg-7.1.1/configure').catch(() => null))) {
      await fs.mkdir('work', {recursive: true});
      await fs.writeFile('work/ffmpeg-7.1.1.tar.xz', await download('https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz'));
      execFileSync('tar', ['-xf', 'work/ffmpeg-7.1.1.tar.xz', '-C', 'work']);
    }
    const readme = await fs.readFile(path.join(dir, 'FFmpeg-README.txt'), 'utf8').catch(() => '');
    if (!readme.includes('FFmpeg 7.1.1.')) execFileSync('sh', ['scripts/build-ffmpeg-mac.sh', target.endsWith('arm64') ? 'arm64' : 'x64'], {stdio: 'inherit'});
  }
  await fs.writeFile(path.join(dir, 'yt-dlp-LICENSE.txt'), await download(`https://raw.githubusercontent.com/yt-dlp/yt-dlp/${version}/LICENSE`));
  await fs.writeFile(path.join(dir, 'yt-dlp-THIRD_PARTY_LICENSES.txt'), await download(`https://raw.githubusercontent.com/yt-dlp/yt-dlp/${version}/THIRD_PARTY_LICENSES.txt`));

  const denoTarget = win ? 'x86_64-pc-windows-msvc' : `${target.endsWith('arm64') ? 'aarch64' : 'x86_64'}-apple-darwin`;
  const archive = path.join(dir, 'deno.zip');
  if (!(await fs.stat(path.join(dir, win ? 'deno.exe' : 'deno')).catch(() => null))) {
    await fs.writeFile(archive, await download(`https://github.com/denoland/deno/releases/download/v2.9.6/deno-${denoTarget}.zip`));
    execFileSync('tar', ['-xf', archive, '-C', dir]);
    await fs.unlink(archive);
  }
  await fs.writeFile(path.join(dir, 'deno-LICENSE.txt'), await download('https://raw.githubusercontent.com/denoland/deno/v2.9.6/LICENSE.md'));
  const hashes = {};
  for (const tool of ['yt-dlp', 'ffmpeg', 'ffprobe', 'deno']) {
    hashes[tool] = createHash('sha256').update(await fs.readFile(path.join(dir, tool + (win ? '.exe' : '')))).digest('hex');
  }
  await fs.writeFile(path.join(dir, 'versions.json'), JSON.stringify({ ytDlp: version, ffmpeg: win ? '6.1.1' : '7.1.1', deno: '2.9.6', target, sha256: hashes }, null, 2));
  console.log(`Ready: ${target}`);
}
