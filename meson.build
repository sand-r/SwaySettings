project(
  'SwaySettings',
  version: '0.5.0',
  meson_version: '>= 0.62.0',
  default_options: ['prefix=/usr'],
)

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
sysconfdir = get_option('sysconfdir')

dep_systemd = dependency('systemd', required: false)

# ── Cargo build ──────────────────────────────────────────────────────────────

cargo = find_program('cargo', required: true)

buildtype = get_option('buildtype')
cargo_profile = buildtype == 'release' ? 'release' : 'dev'
cargo_target_dir = meson.project_build_root() / 'cargo-target'
cargo_output_dir = cargo_target_dir / (cargo_profile == 'release' ? 'release' : 'debug')

cargo_env = environment()
cargo_env.set('CARGO_TARGET_DIR', cargo_target_dir)

# libclang for bindgen (needed by wireplumber-sys etc.)
# Try to find libclang automatically; can be overridden via env
libclang_path = run_command(
  'sh', '-c', 'echo ${LIBCLANG_PATH:-}', check: false,
).stdout().strip()
if libclang_path == ''
  # Common locations
  foreach p : ['/usr/lib/sdk/llvm21/lib', '/usr/lib64', '/usr/lib/llvm/lib', '/usr/lib']
    if run_command('test', '-f', p / 'libclang.so', check: false).returncode() == 0
      libclang_path = p
      break
    endif
  endforeach
endif
if libclang_path != ''
  cargo_env.set('LIBCLANG_PATH', libclang_path)
  cargo_env.set('BINDGEN_EXTRA_CLANG_ARGS', '-I' + libclang_path / 'clang' / run_command(
    'sh', '-c', 'ls ' + libclang_path / 'clang' + ' | head -1', check: false,
  ).stdout().strip() / 'include')
endif

cargo_args = ['build', '--workspace']
if cargo_profile == 'release'
  cargo_args += ['--release']
endif

# Allow excluding crates that need host-only deps (e.g. pam for locker)
cargo_exclude = get_option('cargo_exclude')
foreach exclude : cargo_exclude
  cargo_args += ['--exclude', exclude]
endforeach

# Build all workspace crates via cargo
cargo_build = custom_target(
  'cargo-build',
  output: 'cargo-build-stamp',
  command: [
    'sh', '-c',
    'cd @0@ && @1@ @2@ && touch @3@'.format(
      meson.project_source_root(),
      cargo.full_path(),
      ' '.join(cargo_args),
      '@OUTPUT@',
    ),
  ],
  env: cargo_env,
  console: true,
  build_by_default: true,
  build_always_stale: true,
)

# ── Binary names ─────────────────────────────────────────────────────────────

# Map binary name -> cargo crate name (for exclusion matching)
bin_crate_map = {
  'swaysettings':                       'swaysettings',
  'sway-wallpaper':                     'sway-wallpaper',
  'sway-autostart':                     'sway-autostart',
  'swaysettings-locker':                'swaysettings-locker',
  'swaysettings-screenshot':            'swaysettings-screenshot',
  'swaysettings-upower-monitor':        'swaysettings-upower-monitor',
}

# Install binaries (skip excluded crates)
foreach bin, crate : bin_crate_map
  if crate not in cargo_exclude
    meson.add_install_script(
      'sh', '-c',
      'install -Dm755 "@0@" "@1@"'.format(
        cargo_output_dir / bin,
        '${DESTDIR}' + bindir / bin,
      ),
    )
  endif
endforeach

# ── Data files ───────────────────────────────────────────────────────────────

i18n = import('i18n')

# Desktop file
i18n.merge_file(
  input: 'data/org.erikreider.swaysettings.desktop.in',
  output: 'org.erikreider.swaysettings.desktop',
  type: 'desktop',
  po_dir: 'po',
  install: true,
  install_dir: datadir / 'applications',
)

# Appdata
i18n.merge_file(
  input: 'data/org.erikreider.swaysettings.appdata.xml.in',
  output: 'org.erikreider.swaysettings.appdata.xml',
  po_dir: 'po',
  install: true,
  install_dir: datadir / 'appdata',
)

# GSettings schema
install_data(
  'data/org.erikreider.swaysettings.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

# Application icons
install_data(
  'data/icons/hicolor/scalable/apps/org.erikreider.swaysettings.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)
install_data(
  'data/icons/hicolor/symbolic/apps/org.erikreider.swaysettings-symbolic.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'symbolic' / 'apps',
)

# PAM configuration
install_data(
  'pam/swaysettings-locker',
  install_dir: sysconfdir / 'pam.d',
)

# ── Templated service files ──────────────────────────────────────────────────

conf_data = configuration_data()
conf_data.set('bindir', bindir)

# Resolve systemd user unit directory
systemduserunitdir = get_option('systemduserunitdir')
if systemduserunitdir == ''
  if dep_systemd.found()
    systemduserunitdir = dep_systemd.get_variable(
      'systemduserunitdir',
      pkgconfig_define: ['prefix', prefix],
    )
  else
    systemduserunitdir = prefix / 'lib' / 'systemd' / 'user'
  endif
endif

# Wallpaper daemon systemd service
configure_file(
  input: 'services/systemd/sway-wallpaper.service.in',
  output: 'sway-wallpaper.service',
  configuration: conf_data,
  install: true,
  install_dir: systemduserunitdir,
)

# UPower monitor systemd service
configure_file(
  input: 'services/systemd/swaysettings-upower-monitor.service.in',
  output: 'swaysettings-upower-monitor.service',
  configuration: conf_data,
  install: true,
  install_dir: systemduserunitdir,
)

# ── Post-install ─────────────────────────────────────────────────────────────

meson.add_install_script('build-aux/meson/postinstall.py')
