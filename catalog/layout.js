/* Pure ordering/navigation rules, shared with the regression tests. */
(function (root) {
  'use strict';
  function signature(config) {
    return JSON.stringify([config.dockApps || [], config.preferredApps || [], config.hiddenApps || []]);
  }
  function ordered(apps, config, saved, identify) {
    var hidden = config.hiddenApps || [];
    var pool = apps.filter(function (app, index) {
      return app && typeof app.id === 'string' && hidden.indexOf(app.id) < 0 &&
        apps.findIndex(function (other) { return other && other.id === app.id; }) === index;
    });
    var result = [];
    function take(predicate) {
      var index = pool.findIndex(predicate);
      if (index >= 0) result.push(pool.splice(index, 1)[0]);
    }
    if (saved && saved.signature === signature(config) && Array.isArray(saved.ids)) {
      saved.ids.forEach(function (id) { take(function (app) { return app.id === id; }); });
    } else {
      (config.dockApps || []).forEach(function (id) { take(function (app) { return app.id === id; }); });
      (config.preferredApps || []).forEach(function (key) {
        take(function (app) { var brand = identify(app); return brand && brand.key === key; });
      });
    }
    return result.concat(pool.sort(function (a, b) {
      return (a.title || a.id).localeCompare(b.title || b.id);
    }));
  }
  function next(index, direction, count, columns) {
    if (!count) return -1;
    if (direction === 'left') return index % columns ? index - 1 : index;
    if (direction === 'right') return index % columns < columns - 1 ? Math.min(index + 1, count - 1) : index;
    if (direction === 'up') return index < columns ? -1 : index - columns;
    if (direction === 'down' && Math.floor(index / columns) < Math.floor((count - 1) / columns)) {
      return Math.min(index + columns, count - 1);
    }
    return index;
  }
  root.HomeLayout = { signature: signature, ordered: ordered, next: next };
  if (typeof module !== 'undefined') module.exports = root.HomeLayout;
})(typeof window !== 'undefined' ? window : this);
