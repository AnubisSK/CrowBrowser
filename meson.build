project('crow-browser', ['vala', 'c'],
  version: '1.0.0',
  meson_version: '>= 0.62.0',
  default_options: ['warning_level=2']
)

gnome = import('gnome')

deps = [
  dependency('gtk4'),
  dependency('libadwaita-1', version: '>= 1.5'),
  dependency('webkitgtk-6.0'),
  dependency('gee-0.8'),
  dependency('json-glib-1.0'),
]

resources = gnome.compile_resources(
  'crow-browser-resources',
  'data/crow-browser.gresource.xml',
  source_dir: 'data',
  c_name: 'crow_browser_resources',
)

sources = files(
  'src/main.vala',
  'src/application.vala',
  'src/crypto-utils.vala',
  'src/settings-manager.vala',
  'src/settings-dialog.vala',
  'src/history-manager.vala',
  'src/history-dialog.vala',
  'src/download-manager.vala',
  'src/downloads-dialog.vala',
  'src/adblock.vala',
  'src/extension-manager.vala',
  'src/web-tab.vala',
  'src/browser-window.vala',
  'src/video-downloader.vala',
  'src/update-manager.vala',
)

executable('crow-browser', sources, resources,
  dependencies: deps,
  install: true,
)
