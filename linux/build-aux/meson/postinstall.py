#!/usr/bin/env python3
"""Post-install script — mostly superseded by gnome.post_install()."""
import os
import subprocess

prefix = os.environ.get('MESON_INSTALL_PREFIX', '/usr/local')
datadir = os.path.join(prefix, 'share')

if not os.environ.get('DESTDIR'):
    print('Updating icon cache…')
    subprocess.run(['gtk-update-icon-cache', '-qtf',
                    os.path.join(datadir, 'icons', 'hicolor')], check=False)
    print('Updating desktop database…')
    subprocess.run(['update-desktop-database', '-q',
                    os.path.join(datadir, 'applications')], check=False)
