// ffwd homepage: fills in the newest release and its download links.
// Without this script (or without network) every link goes to the releases page.
(function () {
  'use strict';

  var API = 'https://api.github.com/repos/rennerdo30/ffwd/releases?per_page=30';
  var DOWNLOADS = 'https://github.com/rennerdo30/ffwd/releases/download/';

  // SemVer order for "0.2.0" and "0.2.0-beta.7".
  function parse(tag) {
    var m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.]+))?$/.exec(tag || '');
    if (!m) return null;
    return { n: [+m[1], +m[2], +m[3]], pre: m[4] ? m[4].split('.') : [] };
  }
  function compare(a, b) {
    for (var i = 0; i < 3; i++) if (a.n[i] !== b.n[i]) return a.n[i] - b.n[i];
    if (!a.pre.length || !b.pre.length) return (a.pre.length ? 0 : 1) - (b.pre.length ? 0 : 1);
    for (var j = 0; j < Math.min(a.pre.length, b.pre.length); j++) {
      var x = a.pre[j], y = b.pre[j];
      if (x === y) continue;
      var nx = /^\d+$/.test(x), ny = /^\d+$/.test(y);
      if (nx && ny) return +x - +y;
      if (nx !== ny) return nx ? -1 : 1;
      return x < y ? -1 : 1;
    }
    return a.pre.length - b.pre.length;
  }

  // The newest published release, betas included: ffwd has no stable release yet.
  function newest(releases) {
    var best = null, bestV = null;
    releases.forEach(function (r) {
      if (r.draft) return;
      var v = parse(r.tag_name);
      if (v && (!bestV || compare(v, bestV) > 0)) { best = r; bestV = v; }
    });
    return best;
  }

  // Assets are named ffwd-<version>-<suffix>; only links into this
  // repository's downloads are taken.
  function asset(r, suffix) {
    var list = r.assets || [];
    for (var i = 0; i < list.length; i++) {
      var a = list[i];
      var name = a.name || '';
      var match = suffix === 'SHA256SUMS.txt' ? name === suffix : /^ffwd-/.test(name) && name.slice(-suffix.length - 1) === '-' + suffix;
      if (match && String(a.browser_download_url).indexOf(DOWNLOADS) === 0) return a.browser_download_url;
    }
    return null;
  }

  function markOwnSystem() {
    var p = ((navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent || '').toLowerCase();
    var os = /win/.test(p) ? 'windows' : /mac|iphone|ipad/.test(p) ? 'mac' : /linux|x11|cros/.test(p) ? 'linux' : null;
    if (!os) return;
    var row = document.querySelector('.assets [data-os="' + os + '"]');
    if (row) row.setAttribute('data-mine', '');
  }

  function fill(r) {
    document.querySelectorAll('[data-release="latest"]').forEach(function (el) {
      var field = el.getAttribute('data-field');
      if (field === 'missing') el.hidden = !!r;
      if (field === 'found') el.hidden = !r;
      if (!r) return;
      var version = r.tag_name.replace(/^v/, '');
      if (field === 'version') el.textContent = version;
      if (field === 'kind') el.textContent = r.prerelease ? ' (beta)' : '';
      if (field === 'date') el.textContent = new Date(r.published_at).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' });
      if (field === 'hero-label') el.textContent = 'Download ffwd ' + version;
    });
    if (!r) return;
    document.querySelectorAll('[data-asset]').forEach(function (el) {
      var url = asset(r, el.getAttribute('data-asset'));
      el.href = url || r.html_url;
    });
  }

  markOwnSystem();
  if (!window.fetch) return;
  fetch(API, { headers: { Accept: 'application/vnd.github+json' } })
    .then(function (res) { return res.ok ? res.json() : []; })
    .then(function (list) { fill(newest(Array.isArray(list) ? list : [])); })
    .catch(function () { /* keep the static links */ });
})();
