// Google Tag Manager (GTM-5ZZ3Q3X7), loaded only after the visitor accepts
// cookies (GDPR). Until then Consent Mode v2 stays "denied" and nothing from
// Google loads. Google Analytics is configured as a tag inside GTM.
window.GTM_ID = 'GTM-5ZZ3Q3X7';
window.dataLayer = window.dataLayer || [];
function gtag() { dataLayer.push(arguments); }
gtag('consent', 'default', { ad_storage: 'denied', ad_user_data: 'denied', ad_personalization: 'denied', analytics_storage: 'denied' });
// Custom events go to the data layer; GTM forwards them once it's loaded.
window.track = function (name, params) { if (window.gtmLoaded) dataLayer.push(Object.assign({ event: name }, params || {})); };
window.loadAnalytics = function () {
  if (!window.GTM_ID || window.gtmLoaded) return;
  window.gtmLoaded = true;
  gtag('consent', 'update', { analytics_storage: 'granted' });
  // Google's GTM loader.
  (function (w, d, s, l, i) {
    w[l] = w[l] || []; w[l].push({ 'gtm.start': new Date().getTime(), event: 'gtm.js' });
    var f = d.getElementsByTagName(s)[0], j = d.createElement(s), dl = l != 'dataLayer' ? '&l=' + l : '';
    j.async = true; j.src = 'https://www.googletagmanager.com/gtm.js?id=' + i + dl; f.parentNode.insertBefore(j, f);
  })(window, document, 'script', 'dataLayer', window.GTM_ID);
};
try { if (localStorage.getItem('consent') === 'granted') window.loadAnalytics(); } catch (e) {}

(function () {
  const $ = id => document.getElementById(id);
  if ($('year')) $('year').textContent = new Date().getFullYear();

  // Point the download buttons at the newest release's assets and highlight
  // the visitor's platform. Without the API they keep linking to the releases page.
  const ua = navigator.userAgent;
  const os = /Mac/.test(ua) ? 'mac' : /Win/.test(ua) ? 'win' : /Linux/.test(ua) ? (/aarch64|arm/i.test(ua) ? 'linux-arm' : 'linux') : null;
  const names = { mac: 'macOS', win: 'Windows', linux: 'Linux', 'linux-arm': 'Linux ARM' };
  // Every download link (whole cards for macOS/Windows, the arch buttons for Linux).
  const cards = Array.from(document.querySelectorAll('.downloads [data-match]'));
  const mine = cards.find(c => c.dataset.os === os);
  if (mine) {
    mine.closest('.dl').classList.add('recommended');
    if (!mine.classList.contains('dl')) mine.classList.add('mine');
    if ($('hero-download-label')) $('hero-download-label').textContent = 'Download for ' + names[os];
  }
  // Install steps for the visitor's platform first (download page).
  const steps = os && document.querySelector('.steps [data-os="' + (os === 'linux-arm' ? 'linux' : os) + '"]');
  if (steps) steps.parentElement.prepend(steps);

  if (cards.length) {
    fetch('https://api.github.com/repos/KumaVolt/GitChrono/releases/latest')
      .then(r => (r.ok ? r.json() : Promise.reject(r.status)))
      .then(release => {
        for (const card of cards) {
          const asset = release.assets.find(a => a.name.endsWith(card.dataset.match));
          if (!asset) continue;
          card.href = asset.browser_download_url;
          const size = (asset.size / 1048576).toFixed(1) + ' MB';
          const file = card.querySelector('.file');
          if (file) file.textContent = (asset.name.endsWith('.tar.gz') ? 'tar.gz' : asset.name.split('.').pop()) + ' · ' + size;
          else card.textContent = card.dataset.label + ' · ' + size;
        }
        if (mine && mine.href.includes('/download/') && $('hero-download')) $('hero-download').href = mine.href;
        const date = new Date(release.published_at).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' });
        if ($('release-meta')) {
          $('release-meta').innerHTML =
            'Latest: <a href="' + release.html_url + '">' + release.tag_name + '</a> · ' + date +
            ' · <a href="/changelog/">what\'s new</a> · <a href="https://github.com/KumaVolt/GitChrono/releases">all releases</a>';
        }
        if ($('hero-fine')) $('hero-fine').textContent = release.tag_name + ' · macOS · Windows · Linux — free';
      })
      .catch(() => {
        if ($('release-meta')) {
          $('release-meta').innerHTML = 'First release coming soon — follow along on <a href="https://github.com/KumaVolt/GitChrono">GitHub</a>.';
        }
      });
  }

  // Consent banner (only shown when GTM is configured).
  const banner = $('consent');
  if (banner) {
    let choice = null;
    try { choice = localStorage.getItem('consent'); } catch (e) {}
    const decide = value => {
      try { localStorage.setItem('consent', value); } catch (e) {}
      banner.hidden = true;
      if (value === 'granted') window.loadAnalytics();
    };
    if (window.GTM_ID && !choice) banner.hidden = false;
    $('consent-yes').onclick = () => decide('granted');
    $('consent-no').onclick = () => decide('denied');
  }
  if ($('cookie-settings')) {
    $('cookie-settings').onclick = e => {
      e.preventDefault();
      if (window.GTM_ID && banner) banner.hidden = false;
    };
  }

  // Events: downloads per platform, GitHub clicks, FAQ opens.
  for (const card of cards) {
    card.addEventListener('click', () => track('file_download', { platform: card.dataset.os, link_url: card.href }));
  }
  if ($('hero-download')) $('hero-download').addEventListener('click', () => track('hero_download_click', { platform: os || 'unknown' }));
  document.querySelectorAll('a[href^="https://github.com/KumaVolt/GitChrono"]').forEach(a =>
    a.addEventListener('click', () => track('github_click', { link_url: a.href })));
  document.querySelectorAll('.faq details').forEach(d =>
    d.addEventListener('toggle', () => d.open && track('faq_open', { question: d.querySelector('summary').textContent })));
})();
