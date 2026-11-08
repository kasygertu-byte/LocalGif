// ==UserScript==
// @name         Reddit Post Downloader (with comments)
// @namespace    https://github.com/kasygertu-byte/LocalGif
// @version      1.0.0
// @description  Adds a floating panel to Reddit post pages to download the full post and every comment as HTML, Markdown or JSON.
// @match        https://www.reddit.com/*
// @match        https://old.reddit.com/*
// @match        https://new.reddit.com/*
// @match        https://sh.reddit.com/*
// @grant        GM_addStyle
// @run-at       document-idle
// ==/UserScript==

(function () {
  'use strict';

  // ---------- settings ----------
  const REQUEST_DELAY_MS = 1100;   // pause between "load more" requests (Reddit rate limits)
  const MORE_BATCH = 100;          // max ids per /api/morechildren request

  // ---------- helpers ----------
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  function getPostId() {
    const m = location.pathname.match(/\/comments\/([a-z0-9]+)/i);
    return m ? m[1] : null;
  }

  async function getJSON(url) {
    const res = await fetch(url, { credentials: 'include', headers: { Accept: 'application/json' } });
    if (res.status === 429) {
      setStatus('Rate limited, waiting 10s...');
      await sleep(10000);
      return getJSON(url);
    }
    if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
    return res.json();
  }

  function escapeHtml(s) {
    return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  }

  function fmtDate(utc) {
    return utc ? new Date(utc * 1000).toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC') : '';
  }

  function safeName(s) {
    return String(s).replace(/[\\/:*?"<>|\s]+/g, '_').replace(/_+/g, '_').slice(0, 80);
  }

  function download(content, filename, mime) {
    const blob = new Blob([content], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 5000);
  }

  // ---------- data fetching ----------
  function makeComment(d) {
    return {
      id: d.id,
      name: d.name,
      parent_id: d.parent_id,
      author: d.author,
      score: d.score,
      created_utc: d.created_utc,
      edited: d.edited,
      is_submitter: d.is_submitter,
      distinguished: d.distinguished,
      stickied: d.stickied,
      permalink: d.permalink ? 'https://www.reddit.com' + d.permalink : null,
      body: d.body,
      body_html: d.body_html,
      replies: [],
    };
  }

  // Walks a listing of comments, registering every comment in `map` and queueing "more" stubs.
  function ingestListing(children, parentArray, map, moreQueue) {
    for (const child of children || []) {
      if (child.kind === 't1') {
        const c = makeComment(child.data);
        map.set(c.name, c);
        parentArray.push(c);
        const r = child.data.replies;
        if (r && r.data && r.data.children) ingestListing(r.data.children, c.replies, map, moreQueue);
      } else if (child.kind === 'more') {
        moreQueue.push(child.data);
      }
    }
  }

  // Adds a flat list of things (from /api/morechildren) to the tree using parent_id.
  function ingestFlat(things, rootArray, map, moreQueue) {
    for (const t of things || []) {
      if (t.kind === 't1') {
        const c = makeComment(t.data);
        if (map.has(c.name)) continue;
        map.set(c.name, c);
        const parent = map.get(c.parent_id);
        (parent ? parent.replies : rootArray).push(c);
      } else if (t.kind === 'more') {
        moreQueue.push(t.data);
      }
    }
  }

  async function fetchPost(loadAll) {
    const postId = getPostId();
    if (!postId) throw new Error('Not a post page.');
    const origin = location.origin;
    setStatus('Fetching post...');
    const data = await getJSON(`${origin}/comments/${postId}.json?limit=500&raw_json=1`);
    const p = data[0].data.children[0].data;

    const post = {
      id: p.id,
      name: p.name,
      subreddit: p.subreddit,
      title: p.title,
      author: p.author,
      score: p.score,
      upvote_ratio: p.upvote_ratio,
      num_comments: p.num_comments,
      created_utc: p.created_utc,
      permalink: 'https://www.reddit.com' + p.permalink,
      url: p.url,
      domain: p.domain,
      is_self: p.is_self,
      over_18: p.over_18,
      link_flair_text: p.link_flair_text,
      selftext: p.selftext,
      selftext_html: p.selftext_html,
      media: collectMedia(p),
    };

    const comments = [];
    const map = new Map();
    const moreQueue = [];
    ingestListing(data[1].data.children, comments, map, moreQueue);

    let requests = 0;
    while (loadAll && moreQueue.length) {
      const more = moreQueue.shift();
      if (more.children && more.children.length) {
        // Normal "load more comments" stub
        for (let i = 0; i < more.children.length; i += MORE_BATCH) {
          const ids = more.children.slice(i, i + MORE_BATCH).join(',');
          await sleep(REQUEST_DELAY_MS);
          requests++;
          setStatus(`Loading more comments... (${map.size} loaded, ${moreQueue.length} pending, req ${requests})`);
          const r = await getJSON(`${origin}/api/morechildren.json?api_type=json&raw_json=1&link_id=${post.name}&children=${ids}`);
          ingestFlat(r?.json?.data?.things, comments, map, moreQueue);
        }
      } else if (more.parent_id && more.parent_id.startsWith('t1_')) {
        // "Continue this thread" stub: fetch the sub-thread directly
        const parentId = more.parent_id.slice(3);
        await sleep(REQUEST_DELAY_MS);
        requests++;
        setStatus(`Loading thread continuation... (${map.size} loaded, ${moreQueue.length} pending, req ${requests})`);
        const r = await getJSON(`${origin}/comments/${postId}/_/${parentId}.json?limit=500&raw_json=1`);
        const top = r[1]?.data?.children?.[0];
        const parent = map.get(more.parent_id);
        if (top && parent && top.data?.replies?.data) {
          const tmp = [];
          ingestListing(top.data.replies.data.children, tmp, map, moreQueue);
          parent.replies.push(...tmp);
        }
      }
    }

    setStatus(`Done: ${map.size} comments.`);
    return { post, comments, comment_count: map.size, downloaded_utc: Math.floor(Date.now() / 1000) };
  }

  function collectMedia(p) {
    const out = [];
    if (p.is_gallery && p.media_metadata) {
      const order = p.gallery_data?.items?.map((i) => i.media_id) || Object.keys(p.media_metadata);
      for (const id of order) {
        const m = p.media_metadata[id];
        const src = m?.s?.u || m?.s?.gif || m?.s?.mp4;
        if (src) out.push({ type: m.e === 'AnimatedImage' ? 'gif' : 'image', url: src });
      }
    } else if (p.secure_media?.reddit_video?.fallback_url) {
      out.push({ type: 'video', url: p.secure_media.reddit_video.fallback_url });
    } else if (p.post_hint === 'image' && p.url) {
      out.push({ type: 'image', url: p.url });
    } else if (p.preview?.images?.[0]?.source?.url) {
      out.push({ type: 'preview', url: p.preview.images[0].source.url });
    }
    return out;
  }

  // ---------- exporters ----------
  function toMarkdown(d) {
    const p = d.post;
    const lines = [];
    lines.push(`# ${p.title}`, '');
    lines.push(`**r/${p.subreddit}** · u/${p.author} · ${p.score} points · ${fmtDate(p.created_utc)}`);
    lines.push(`Link: ${p.permalink}`);
    if (!p.is_self && p.url) lines.push(`URL: ${p.url}`);
    lines.push('');
    for (const m of p.media) lines.push(m.type === 'video' ? `[video](${m.url})` : `![${m.type}](${m.url})`);
    if (p.media.length) lines.push('');
    if (p.selftext) lines.push(p.selftext, '');
    lines.push('---', '', `## Comments (${d.comment_count})`, '');

    const walk = (list, depth) => {
      for (const c of list) {
        const q = '> '.repeat(depth);
        lines.push(`${q}**u/${c.author}**${c.is_submitter ? ' (OP)' : ''} · ${c.score} points · ${fmtDate(c.created_utc)}`);
        lines.push(q.trimEnd());
        for (const l of String(c.body ?? '').split('\n')) lines.push(q + l);
        lines.push('');
        walk(c.replies, depth + 1);
      }
    };
    walk(d.comments, 0);
    return lines.join('\n');
  }

  function toHtml(d) {
    const p = d.post;
    const media = p.media.map((m) =>
      m.type === 'video'
        ? `<video controls src="${escapeHtml(m.url)}"></video>`
        : `<a href="${escapeHtml(m.url)}"><img src="${escapeHtml(m.url)}" alt=""></a>`
    ).join('\n');

    const renderComments = (list) => list.map((c) => `
<details class="c" open>
  <summary><b>u/${escapeHtml(c.author)}</b>${c.is_submitter ? ' <span class="op">OP</span>' : ''}
    · ${c.score} pts · <a href="${escapeHtml(c.permalink || '#')}">${escapeHtml(fmtDate(c.created_utc))}</a></summary>
  <div class="body">${c.body_html || escapeHtml(c.body)}</div>
  ${c.replies.length ? `<div class="replies">${renderComments(c.replies)}</div>` : ''}
</details>`).join('');

    return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${escapeHtml(p.title)}</title>
<style>
body{font:15px/1.5 system-ui,sans-serif;max-width:900px;margin:0 auto;padding:16px;background:#fff;color:#1a1a1b}
@media (prefers-color-scheme:dark){body{background:#1a1a1b;color:#d7dadc}a{color:#4fbcff}.c{border-color:#343536!important}}
h1{margin:.2em 0}.meta{color:#787c7e;font-size:13px}
img,video{max-width:100%;margin:8px 0;display:block}
.c{border-left:2px solid #edeff1;padding-left:10px;margin:8px 0}
.c summary{cursor:pointer;font-size:13px;color:#787c7e}
.c .body{margin:4px 0}.replies{margin-left:6px}
.op{background:#0079d3;color:#fff;border-radius:3px;padding:0 4px;font-size:11px}
pre,code{white-space:pre-wrap;word-break:break-word}blockquote{border-left:3px solid #ccc;margin:0;padding-left:8px;color:#787c7e}
</style></head><body>
<div class="meta">r/${escapeHtml(p.subreddit)} · u/${escapeHtml(p.author)} · ${p.score} points · ${escapeHtml(fmtDate(p.created_utc))}</div>
<h1>${escapeHtml(p.title)}</h1>
<div class="meta"><a href="${escapeHtml(p.permalink)}">${escapeHtml(p.permalink)}</a>${!p.is_self && p.url ? ` · <a href="${escapeHtml(p.url)}">${escapeHtml(p.url)}</a>` : ''}</div>
${media}
<div class="post">${p.selftext_html || ''}</div>
<hr><h2>Comments (${d.comment_count})</h2>
${renderComments(d.comments)}
<hr><div class="meta">Saved ${escapeHtml(fmtDate(d.downloaded_utc))}</div>
</body></html>`;
  }

  // ---------- GUI ----------
  GM_addStyle(`
#rpd-panel{position:fixed;right:16px;bottom:16px;z-index:2147483647;background:#1a1a1b;color:#d7dadc;
  border:1px solid #343536;border-radius:10px;padding:10px;font:13px system-ui,sans-serif;box-shadow:0 4px 16px rgba(0,0,0,.4);width:220px}
#rpd-panel.rpd-min{width:auto}
#rpd-panel.rpd-min .rpd-body{display:none}
#rpd-panel .rpd-head{display:flex;justify-content:space-between;align-items:center;font-weight:600;gap:8px}
#rpd-panel .rpd-toggle{background:none;border:none;color:#d7dadc;cursor:pointer;font-size:14px;padding:0 4px}
#rpd-panel .rpd-body{margin-top:8px;display:flex;flex-direction:column;gap:6px}
#rpd-panel button.rpd-btn{background:#ff4500;color:#fff;border:none;border-radius:6px;padding:7px;cursor:pointer;font-weight:600}
#rpd-panel button.rpd-btn:disabled{opacity:.5;cursor:default}
#rpd-panel label{display:flex;gap:6px;align-items:center;cursor:pointer}
#rpd-panel .rpd-status{font-size:12px;color:#818384;min-height:1em;word-break:break-word}
`);

  let panel, statusEl;

  function setStatus(t) { if (statusEl) statusEl.textContent = t; }

  function buildPanel() {
    panel = document.createElement('div');
    panel.id = 'rpd-panel';
    panel.innerHTML = `
      <div class="rpd-head"><span>Download post</span><button class="rpd-toggle" title="Minimize">–</button></div>
      <div class="rpd-body">
        <button class="rpd-btn" data-fmt="html">HTML</button>
        <button class="rpd-btn" data-fmt="md">Markdown</button>
        <button class="rpd-btn" data-fmt="json">JSON</button>
        <label><input type="checkbox" class="rpd-all" checked> Load all hidden comments</label>
        <div class="rpd-status"></div>
      </div>`;
    document.body.appendChild(panel);
    statusEl = panel.querySelector('.rpd-status');

    const toggle = panel.querySelector('.rpd-toggle');
    toggle.addEventListener('click', () => {
      panel.classList.toggle('rpd-min');
      toggle.textContent = panel.classList.contains('rpd-min') ? '+' : '–';
    });

    panel.querySelectorAll('.rpd-btn').forEach((btn) => btn.addEventListener('click', () => run(btn.dataset.fmt)));
  }

  let busy = false;
  async function run(fmt) {
    if (busy) return;
    busy = true;
    const btns = panel.querySelectorAll('.rpd-btn');
    btns.forEach((b) => (b.disabled = true));
    try {
      const loadAll = panel.querySelector('.rpd-all').checked;
      const d = await fetchPost(loadAll);
      const base = `${safeName(d.post.subreddit)}_${d.post.id}_${safeName(d.post.title)}`;
      if (fmt === 'html') download(toHtml(d), base + '.html', 'text/html;charset=utf-8');
      else if (fmt === 'md') download(toMarkdown(d), base + '.md', 'text/markdown;charset=utf-8');
      else download(JSON.stringify(d, null, 2), base + '.json', 'application/json;charset=utf-8');
      setStatus(`Saved ${d.comment_count} comments.`);
    } catch (e) {
      console.error('[Reddit Post Downloader]', e);
      setStatus('Error: ' + e.message);
    } finally {
      busy = false;
      btns.forEach((b) => (b.disabled = false));
    }
  }

  // Reddit is a single-page app: show the panel only on post pages and react to navigation.
  let lastPath = null;
  function update() {
    if (location.pathname === lastPath) return;
    lastPath = location.pathname;
    const onPost = !!getPostId();
    if (onPost && !panel) buildPanel();
    if (panel) {
      panel.style.display = onPost ? '' : 'none';
      if (onPost && !busy) setStatus('');
    }
  }
  update();
  setInterval(update, 1000);
})();
