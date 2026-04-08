"""Installer channel routing tests for aim-data deployments."""

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def read(path: str) -> str:
    return (REPO_ROOT / path).read_text()


def test_root_install_sh_passes_channel_to_os_installers():
    content = read("install.sh")

    assert 'CHANNEL="${VECTORAIZ_CHANNEL:-direct}"' in content
    assert 'bash -s -- --channel "$CHANNEL"' in content
    assert "install-vectoraiz.ps1 -OutFile \\$tmp" in content
    assert "& \\$tmp -Channel '$CHANNEL'" in content


def test_unix_installers_select_compose_file_by_channel():
    for path in ("installers/mac/install-mac.sh", "installers/linux/install-linux.sh"):
        content = read(path)

        assert '[ "$CHANNEL" = "aim-data" ]' in content
        assert 'COMPOSE_FILE="docker-compose.aim-data.yml"' in content
        assert 'COMPOSE_FILE="docker-compose.customer.yml"' in content
        assert 'VECTORAIZ_CHANNEL=${CHANNEL}' in content


def test_windows_installers_select_compose_file_by_channel():
    for path in ("installers/windows/install-vectoraiz.ps1", "install.ps1"):
        content = read(path)

        assert '"aim-data"' in content
        assert 'docker-compose.aim-data.yml' in content
        assert 'docker-compose.customer.yml' in content
        assert 'VECTORAIZ_CHANNEL=$Channel' in content
