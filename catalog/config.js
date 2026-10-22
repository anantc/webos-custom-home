// Reinstall after changing this file. Actual launch IDs always come from your TV.
window.CONFIG = {
  // Brand keys are matched against synced titles/IDs, never used as launch IDs.
  // Apps fill a continuous grid, with the first three rows visible at six columns.
  preferredApps: ['apple-tv', 'netflix', 'prime-video', 'jiohotstar', 'youtube', 'spotify', 'zee5', 'sonyliv'],
  dockApps: [], // Optional exact TV app IDs to place first in the top-row tray.
  // Exact TV app IDs to exclude from Home.
  // Add any number of IDs here, then rebuild and reinstall.
  hiddenApps: [
    'com.palmdts.devmode', // Developer Mode
    'com.webos.app.livetv',
    'com.webos.app.hdmi1', 'com.webos.app.hdmi2', 'com.webos.app.hdmi3', 'com.webos.app.hdmi4',
    'shortcutcopilot'
  ]
};
