const read = () => {
  const page = location.href;
  const id = page.match(/\/video\/(\d+)/)?.[1];
  if (!id || !location.hostname.endsWith("douyin.com")) return { error: "请打开一条抖音视频详情页。" };
  const playing = [...document.querySelectorAll("video")].filter(video => {
    const box = video.getBoundingClientRect();
    return video.readyState >= 1 && box.width * box.height && box.bottom > 0 && box.top < innerHeight;
  });
  if (playing.length !== 1) return { error: playing.length ? "请打开单条视频详情页后再发送。" : "请先播放视频，再发送到拾影。" };
  const resources = performance.getEntriesByType("resource")
    .map(entry => entry.name)
    .filter(name => name.startsWith("https://") && /(^|\.)douyinvod\.com$/.test(new URL(name).hostname));
  const media = resources.filter(name => new URL(name).pathname.includes("/media-video-")).at(-1);
  const marker = media && new URL(media).searchParams.get("l");
  const audio = resources.filter(name => new URL(name).pathname.includes("/media-audio-") && new URL(name).searchParams.get("l") === marker).at(-1);
  if (!media || !marker || !audio) return { error: "未找到同一视频的画面和声音地址，请刷新详情页、播放几秒后重试。" };
  const video = playing[0];
  return { id, page, title: document.title.replace(/\s+-\s+抖音$/, "").slice(0, 120), media, audio, duration: video.duration || 0, width: video.videoWidth, height: video.videoHeight };
};
chrome.action.onClicked.addListener(async tab => {
  if (!tab.id) return;
  let result;
  try { [result] = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: read }); }
  catch { return; }
  const data = result?.result;
  if (!data || data.error) return alertFallback(tab.id, data?.error || "无法读取当前视频。");
  const query = new URLSearchParams({ v: "2", id: data.id, page: data.page, title: data.title, media: data.media, audio: data.audio, duration: String(data.duration), width: String(data.width), height: String(data.height) });
  const link = "shiying://download?" + query.toString();
  if (link.length > 16 * 1024) return alertFallback(tab.id, "视频信息过长，无法发送。");
  await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: href => { location.href = href; }, args: [link] });
});
function alertFallback(tabId, text) {
  chrome.scripting.executeScript({ target: { tabId }, func: message => alert(message), args: [text] });
}
