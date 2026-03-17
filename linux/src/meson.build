cargo = find_program('cargo', required: true)

profile = get_option('profile')
is_devel = profile == 'development'

if is_devel
  rust_target = 'debug'
  cargo_args = []
else
  rust_target = 'release'
  cargo_args = ['--release']
endif

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'tunnel',
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  command: [
    find_program(meson.project_source_root() / 'build-aux' / 'meson' / 'cargo_wrapper.py'),
    '--manifest-path', meson.project_source_root() / 'Cargo.toml',
    '--bin', 'tunnel',
    '--target-dir', meson.project_source_root() / 'target',
    cargo_args,
    '@OUTPUT@',
  ],
)
