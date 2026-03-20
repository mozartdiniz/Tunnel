project(
  'tunnel',
  version: '0.1.0',
  meson_version: '>= 1.3.0',
)

i18n = import('i18n')
gnome = import('gnome')

profile = get_option('profile')
if profile == 'development'
  app_id = 'dev.tunnel.Tunnel.Devel'
else
  app_id = 'dev.tunnel.Tunnel'
endif

subdir('data')
subdir('po')
subdir('src')

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
