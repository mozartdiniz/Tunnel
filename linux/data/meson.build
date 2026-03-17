profile = get_option('profile')
app_id = profile == 'development' ? 'dev.tunnel.Tunnel.Devel' : 'dev.tunnel.Tunnel'

# ── Desktop file ─────────────────────────────────────────────────────────────
desktop_file = i18n.merge_file(
  input: 'dev.tunnel.Tunnel.desktop.in',
  output: app_id + '.desktop',
  type: 'desktop',
  po_dir: '../po',
  install: true,
  install_dir: get_option('datadir') / 'applications',
)

desktop_utils = find_program('desktop-file-validate', required: false)
if desktop_utils.found()
  test('Validate desktop file', desktop_utils, args: [desktop_file])
endif

# ── AppStream metainfo ────────────────────────────────────────────────────────
appstream_file = i18n.merge_file(
  input: 'dev.tunnel.Tunnel.metainfo.xml.in',
  output: app_id + '.metainfo.xml',
  po_dir: '../po',
  install: true,
  install_dir: get_option('datadir') / 'metainfo',
)

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test('Validate metainfo', appstreamcli,
    args: ['validate', '--no-net', '--explain', appstream_file],
  )
endif

# ── D-Bus service file ────────────────────────────────────────────────────────
service_conf = configuration_data()
service_conf.set('bindir', get_option('prefix') / get_option('bindir'))

configure_file(
  input: 'dev.tunnel.Tunnel.service.in',
  output: app_id + '.service',
  configuration: service_conf,
  install: true,
  install_dir: get_option('datadir') / 'dbus-1' / 'services',
)

# ── Icons ─────────────────────────────────────────────────────────────────────
install_data(
  'icons/hicolor/scalable/apps/dev.tunnel.Tunnel.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/scalable/apps',
  rename: app_id + '.svg',
)

install_data(
  'icons/hicolor/symbolic/apps/dev.tunnel.Tunnel-symbolic.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/symbolic/apps',
  rename: app_id + '-symbolic.svg',
)
