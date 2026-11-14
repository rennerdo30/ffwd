// ffwd homepage: the screenshot viewer in the hero.
// Real captures of the app, each in ffwd's dark and light theme.
// Without this script the page shows the history in the visitor's colour scheme.
(function () {
  'use strict';

  var root = document.querySelector('.viewer');
  if (!root) return;

  var img = root.querySelector('[data-shot]');
  var picture = img.parentNode;
  var caption = root.querySelector('#viewer-caption');
  var viewButtons = root.querySelectorAll('[data-view]');
  var themeButtons = root.querySelectorAll('[data-theme]');

  var views = {
    history: {
      alt: "ffwd showing a repository's history: the branch graph with coloured lanes, branch and tag labels, the selected commit's details and its diff.",
      caption: 'The history with its branch graph. Select a commit and its header is there at once; files and diff fill in behind it.'
    },
    changes: {
      alt: 'Local changes: unstaged and staged files on the left, the selected file’s diff on the right, and the commit message box.',
      caption: 'Local changes: stage whole files, hunks or single lines, then commit. Hooks show their output.'
    },
    diff: {
      alt: 'A commit’s diff side by side, with word-level highlights and syntax colours.',
      caption: 'Side by side or unified, with word highlights, syntax colours and change marks on the scrollbar.'
    },
    settings: {
      alt: 'The Settings page: sections on the left, the Appearance section with theme, views and layout choices.',
      caption: 'Settings are searchable and apply as you change them, including Git’s own name, email and certificates.'
    }
  };

  var dark = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  var state = { view: 'history', theme: dark && !dark.matches ? 'light' : 'dark', chosenTheme: false };

  // Preload the other captures once the page is idle, so switching is instant.
  function preload() {
    Object.keys(views).forEach(function (v) {
      ['dark', 'light'].forEach(function (t) { (new Image()).src = 'assets/shots/' + v + '-' + t + '.webp'; });
    });
  }

  function show(announce) {
    var v = views[state.view];
    // The script owns the theme from now on; drop the CSS-media fallback source.
    var source = picture.querySelector('source');
    if (source) picture.removeChild(source);
    img.src = 'assets/shots/' + state.view + '-' + state.theme + '.webp';
    img.alt = v.alt;
    caption.textContent = v.caption;
    caption.setAttribute('aria-live', announce ? 'polite' : 'off');
    viewButtons.forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-view') === state.view)); });
    themeButtons.forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-theme') === state.theme)); });
  }

  viewButtons.forEach(function (b) {
    b.addEventListener('click', function () { state.view = b.getAttribute('data-view'); show(true); });
  });
  themeButtons.forEach(function (b) {
    b.addEventListener('click', function () { state.theme = b.getAttribute('data-theme'); state.chosenTheme = true; show(true); });
  });
  if (dark && dark.addEventListener) {
    dark.addEventListener('change', function (e) {
      if (state.chosenTheme) return;
      state.theme = e.matches ? 'dark' : 'light';
      show(false);
    });
  }

  show(false);
  if ('requestIdleCallback' in window) window.requestIdleCallback(preload); else setTimeout(preload, 1500);
})();
