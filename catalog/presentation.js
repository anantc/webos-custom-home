/* Presentation overrides only. Native app IDs and launch parameters are never replaced. */
(function (root) {
  'use strict';
  var brands = [
    { key: 'apple-tv', name: 'Apple TV', match: /apple[\s.+_-]*tv|appletv/i, color: [112, 126, 217], crop: true },
    { key: 'netflix', name: 'Netflix', match: /netflix/i, color: [169, 27, 54] },
    { key: 'prime-video', name: 'Prime Video', match: /prime[\s_-]*video|^amazon$/i, color: [28, 108, 201] },
    { key: 'jiohotstar', name: 'JioHotstar', match: /hotstar/i, color: [90, 63, 208] },
    { key: 'youtube', name: 'YouTube', match: /^(youtube|youtube\.leanback\.v4)$/i, color: [167, 39, 62] },
    { key: 'spotify', name: 'Spotify', match: /spotify/i, color: [30, 136, 89] },
    { key: 'zee5', name: 'ZEE5', match: /zee[\s_-]*5|^z5$/i, color: [102, 89, 153], square: true, background: 'linear-gradient(110deg, #30303d, #10101e)' },
    { key: 'sonyliv', name: 'Sony LIV', match: /sony[\s._-]*liv/i, color: [123, 57, 198] },
    { key: 'lg-channels', name: 'LG Channels', match: /^lg[\s._-]*channels$|^com\.webos\.app\.lgchannels$/i, color: [255, 47, 46] }
  ];
  function forApp(app) {
    for (var i = 0; i < brands.length; i++) {
      if (brands[i].match.test(app.title || '') || brands[i].match.test(app.id || '')) return brands[i];
    }
    return null;
  }
  root.Presentation = { brands: brands, forApp: forApp };
  if (typeof module !== 'undefined') module.exports = root.Presentation;
})(typeof window !== 'undefined' ? window : this);
