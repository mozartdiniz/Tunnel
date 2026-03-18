global_conf = configuration_data()
global_conf.set_quoted('APP_ID', application_id)
global_conf.set_quoted('PKGDATADIR', pkgdatadir)
global_conf.set_quoted('PROFILE', profile)
global_conf.set_quoted('VERSION', version + version_suffix)
global_conf.set_quoted('GETTEXT_PACKAGE', gettext_package)
global_conf.set_quoted('LOCALEDIR', localedir)

cargo_options = [ '--manifest-path', meson.project_source_root() / 'Cargo.toml' ]
cargo_options += [ '--target-dir', meson.project_build_root() / 'src' ]
cargo_options += [ '--no-default-features' ]
cargo_features = [ 'meson' ]

if get_option('profile') == 'default' or get_option('profile') == 'release'
  cargo_options += [ '--release' ]
  rust_target = 'release'
  message('Building in release mode')
else
  rust_target = 'debug'
  message('Building in debug mode')
endif

if qr_code_scanning
  cargo_features += [ 'qr_code_scanning' ]
endif

file_extension = ''
if get_option('target') != ''
  cargo_options += [ '--target', get_option('target') ]
  if get_option('target').contains('windows')
    file_extension = '.exe'
    bindir = ''
  endif
endif

cargo_env = [
    'CARGO_HOME=' + meson.project_build_root() / 'cargo-home',
    'LOCALEDIR=' + get_option('prefix') / get_option('localedir')
]

cargo_options += [ '--features' ] + ','.join(cargo_features)

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name() + file_extension,
  console: true,
  install: true,
  install_dir: bindir,
  depends: appdata_gresource,
  command: [
    'env',
    cargo_env,
    cargo, 'build',
    cargo_options,
    # Replace with --out-dir once stable https://github.com/rust-lang/cargo/issues/6790#issuecomment-655728327=
    '&&', 'cp', 'src' / get_option('target') / rust_target / meson.project_name() + file_extension, '@OUTPUT@'
  ]
)
