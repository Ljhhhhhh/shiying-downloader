const $ = id => document.getElementById(id);
const labels = { preparing: '读取中', downloading: '下载中', merging: '处理中', done: '已完成', cancelled: '已取消', error: '下载失败' };
const qualityLabels = { best: '最佳画质', '1080': '最高 1080p', '720': '最高 720p', audio: 'M4A 音频' };
const activeStatuses = ['preparing', 'downloading', 'merging'];
let jobs = [], busy = false;
const api = window.__TAURI__?.core;
function message(text = '') { $('message').textContent = String(text); $('message').hidden = !text; }
function el(tag, className, text) { const node = document.createElement(tag); node.className = className; if (text !== undefined) node.textContent = text; return node; }
async function invoke(command, args) { if (!api) throw new Error('请在拾影桌面应用中使用下载功能。'); return api.invoke(command, args); }
function controls() { $('start').disabled = busy; $('start').querySelector('span').textContent = busy ? '下载进行中' : '开始下载'; $('clear').disabled = busy || !jobs.length; }
function render() {
  $('jobs').replaceChildren(); $('count').textContent = String(jobs.length); $('total').textContent = String(jobs.length); $('empty').hidden = jobs.length > 0; controls();
  for (const job of jobs) {
    const card = el('article', 'job');
    const top = el('div', 'job-top');
    const icon = el('div', 'job-icon'); icon.setAttribute('aria-hidden', 'true');
    icon.innerHTML = '<svg viewBox="0 0 24 24"><path d="M5 3h10l4 4v14H5Z M14 3v5h5 M10 11l5 3-5 3Z"/></svg>';
    const info = el('div', 'job-info');
    const title = el('div', 'job-title', job.title); title.title = job.title;
    let host; try { host = new URL(job.url).hostname; } catch { host = ''; }
    info.append(title, el('div', 'job-subtitle', `${host} · ${qualityLabels[job.quality] || ''}`));
    const status = el('div', `status ${job.status}`, job.status === 'downloading' ? `下载中 · ${Math.round(job.progress)}%` : (labels[job.status] || job.status));
    top.append(icon, info, status); card.append(top);
    if (activeStatuses.includes(job.status)) {
      const bar = document.createElement('progress'); bar.max = 100;
      if (job.status === 'downloading') bar.value = job.progress;
      bar.setAttribute('aria-label', '下载进度'); card.append(bar);
    }
    const bottom = el('div', 'job-bottom'); bottom.append(el('div', `job-detail ${job.status}`, job.detail));
    const button = el('button', 'text-button', job.status === 'done' ? '显示文件' : activeStatuses.includes(job.status) ? '取消' : '重试');
    button.type = 'button'; button.disabled = !activeStatuses.includes(job.status) && job.status !== 'done' && busy;
    button.addEventListener('click', async () => {
      message(); button.disabled = true;
      try {
        if (job.status === 'done') await invoke('reveal_file', { id: job.id });
        else if (activeStatuses.includes(job.status)) { await invoke('cancel_download', { id: job.id }); button.textContent = '取消中…'; return; }
        else { $('url').value = job.url; $('quality').value = job.quality; await start(job.url, job.quality); }
      } catch (error) { message(error); }
      button.disabled = false;
    });
    bottom.append(button); card.append(bottom); $('jobs').append(card);
  }
}
async function start(url, quality) {
  if (busy) return;
  busy = true; message(); controls();
  try {
    const job = await invoke('start_download', { url, quality });
    if (!jobs.some(j => j.id === job.id)) jobs.unshift(job);
    $('url').value = ''; render();
  } catch (error) { busy = false; controls(); throw error; }
}
$('download-form').addEventListener('submit', async event => { event.preventDefault(); try { await start($('url').value, $('quality').value); } catch (error) { message(error); } });
$('choose').addEventListener('click', async () => {
  try { const directory = await invoke('choose_directory'); if (directory) { await invoke('set_directory', { directory }); $('directory').textContent = directory; $('directory').title = directory; } }
  catch (error) { message(error); }
});
$('open-folder').addEventListener('click', async () => { try { await invoke('reveal_file', { id: null }); } catch (error) { message(error); } });
$('clear').addEventListener('click', async () => { try { await invoke('clear_history'); jobs = []; render(); } catch (error) { message(error); } });
async function init() {
  try {
    if (window.__TAURI__) {
      await window.__TAURI__.event.listen('download', ({ payload: job }) => {
        const index = jobs.findIndex(j => j.id === job.id); if (index < 0) jobs.unshift(job); else jobs[index] = job;
        render();
      });
      await window.__TAURI__.event.listen('idle', () => { busy = false; render(); });
      await window.__TAURI__.event.listen('storage-error', ({ payload }) => message(payload));
    }
    const state = await invoke('snapshot'); jobs = state.jobs; busy = jobs.some(j => activeStatuses.includes(j.status));
    $('directory').textContent = state.directory; $('directory').title = state.directory; render();
  } catch (error) { message(error); $('directory').textContent = '下载文件夹'; }
}
init();
