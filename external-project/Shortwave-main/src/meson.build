# Setup cargo
cargo = find_program('cargo')
cargo_env = [
  'CARGO_HOME=' + meson.project_build_root() / 'cargo-home',
  'MESON_NAME=' + name,
  'MESON_PKGNAME=' + meson.project_name(),
  'MESON_APP_ID=' + app_id,
  'MESON_PATH_ID=' + path_id,
  'MESON_VERSION=' + meson.project_version(),
  'MESON_PROFILE=' + profile,
  'MESON_VCS_TAG=' + vcs_tag,
  'MESON_LOCALEDIR=' + localedir,
  'MESON_DATADIR=' + datadir,
]
cargo_options = [ '--manifest-path', meson.project_source_root() / 'Cargo.toml' ]
cargo_options += [ '--target-dir', meson.project_build_root() / 'target' ]

if get_option('profile') == 'default'
  cargo_options += [ '--release' ]
  rust_target = 'release'
  message('Building in release mode')
else
  rust_target = 'debug'
  message('Building in debug mode')
endif

cargo_release = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: bindir,
  command: [
    'env',
    cargo_env,
    cargo, 'build',
    cargo_options,
    '&&',
    'cp', 'target' / rust_target / meson.project_name(), '@OUTPUT@',
  ]
)
